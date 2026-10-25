//! The regions of a program that a profile measures, and where each sits in the source.
//!
//! The interpreter walks the syntax tree, so the unit of measurement is the statement (and the user
//! function, which is measured as a whole). A region is identified by the UTF-16 offset where the
//! parser anchored it, the identity the debugger's source map uses; a line is only how a region is
//! shown. Statement ranges are start-anchored, so where a region *ends* comes from the parser's
//! record of statement ends (`CompiledProgram::statement_ends`).

use crate::pine::ast::{Expression, Statement};
use crate::pine::program::{CompiledProgram, StatementEnd};
use crate::pine::{SourcePosition, traversal};

/// What a region is, for how it is described and which derived numbers it has.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegionKind {
    /// A declaration, an assignment, an expression statement, `break` or `continue`.
    Statement,
    /// `for` (either form) and `while`: the region's executions are its entries, and its
    /// iterations are those of the first statement of its body.
    Loop,
    /// `if` and `switch`: the first statement of each branch counts how often it was taken.
    Branch,
    /// A user-defined function (or method): measured per call, wherever it is called from.
    Function,
}

impl RegionKind {
    pub fn describe(self) -> &'static str {
        match self {
            Self::Statement => "Statement",
            Self::Loop => "Loop",
            Self::Branch => "Conditional",
            Self::Function => "Function",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    pub kind: RegionKind,
    /// UTF-16 offset of the first token, in the compiled (CRLF-folded) source.
    pub offset: usize,
    pub start: SourcePosition,
    /// Where the statement ends, nested blocks included.
    pub end: SourcePosition,
    /// The region this one is written inside: a loop for its body, a function for its statements,
    /// a declaration for the `if` it takes its value from. Always an earlier index.
    pub parent: Option<u32>,
    /// For each nested block (a loop's body, each branch, each `switch` arm), the region of its
    /// first statement, which runs once per time the block does.
    pub blocks: Vec<Option<u32>>,
    /// The function's name, for a function region.
    pub name: Option<String>,
    /// Whether any region is written inside this one.
    pub has_children: bool,
}

impl Region {
    /// Whether this region reaches over more than the line it starts on.
    pub fn spans_lines(&self) -> bool {
        self.end.line > self.start.line
    }

    /// Shown with a code-block range: it has regions inside it, or is a loop, a branch or a
    /// function spread over several lines.
    pub fn is_block(&self) -> bool {
        self.has_children || (self.kind != RegionKind::Statement && self.spans_lines())
    }
}

/// Every measurable region of one program. Indices are stable: a region comes after the one it is
/// written inside, and a block's statements come in the order they are written.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RegionTable {
    regions: Vec<Region>,
    /// `(start offset, region)`, sorted by offset.
    by_offset: Vec<(usize, u32)>,
    /// `(start offset of a function's first body statement, the function's region)`, sorted.
    function_bodies: Vec<(usize, u32)>,
}

impl RegionTable {
    pub fn build(program: &CompiledProgram) -> Self {
        let mut builder = Builder {
            ends: &program.statement_ends,
            regions: Vec::new(),
        };
        builder.block(&program.statements, None);
        let regions = builder.regions;
        let mut by_offset: Vec<(usize, u32)> = regions
            .iter()
            .enumerate()
            .map(|(index, region)| (region.offset, index as u32))
            .collect();
        by_offset.sort_unstable();
        by_offset.dedup_by_key(|(offset, _)| *offset);
        let mut function_bodies: Vec<(usize, u32)> = regions
            .iter()
            .enumerate()
            .filter(|(_, region)| region.kind == RegionKind::Function)
            .filter_map(|(index, region)| {
                let first = (*region.blocks.first()?)?;
                Some((regions[first as usize].offset, index as u32))
            })
            .collect();
        function_bodies.sort_unstable();
        Self {
            regions,
            by_offset,
            function_bodies,
        }
    }

    /// One statement at offset 0, to measure the cost of measuring.
    pub(super) fn single() -> Self {
        let position = SourcePosition::new(1, 1, 0);
        Self {
            regions: vec![Region {
                kind: RegionKind::Statement,
                offset: 0,
                start: position,
                end: position,
                parent: None,
                blocks: Vec::new(),
                name: None,
                has_children: false,
            }],
            by_offset: vec![(0, 0)],
            function_bodies: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.regions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }

    pub fn regions(&self) -> &[Region] {
        &self.regions
    }

    pub fn get(&self, index: u32) -> Option<&Region> {
        self.regions.get(index as usize)
    }

    /// The region that starts at `offset`.
    #[inline]
    pub fn at_offset(&self, offset: usize) -> Option<u32> {
        self.by_offset
            .binary_search_by_key(&offset, |(start, _)| *start)
            .ok()
            .map(|position| self.by_offset[position].1)
    }

    /// The function whose body starts with the statement at `first_statement`.
    #[inline]
    pub fn function_with_body(&self, first_statement: usize) -> Option<u32> {
        self.function_bodies
            .binary_search_by_key(&first_statement, |(start, _)| *start)
            .ok()
            .map(|position| self.function_bodies[position].1)
    }

    /// Whether `ancestor` is `region` or encloses it.
    pub fn encloses(&self, ancestor: u32, region: u32) -> bool {
        let mut next = Some(region);
        while let Some(index) = next {
            if index == ancestor {
                return true;
            }
            next = self.regions[index as usize].parent;
        }
        false
    }
}

struct Builder<'a> {
    ends: &'a [StatementEnd],
    regions: Vec<Region>,
}

impl Builder<'_> {
    /// Adds the regions of a block's statements. Returns the first one added: the block's first
    /// statement, which runs each time the block does.
    fn block(&mut self, statements: &[Statement], parent: Option<u32>) -> Option<u32> {
        let first = self.regions.len();
        for statement in statements {
            self.statement(statement, parent);
        }
        (self.regions.len() > first).then_some(first as u32)
    }

    fn end_of(&self, offset: usize) -> Option<SourcePosition> {
        self.ends
            .binary_search_by_key(&offset, |end| end.offset)
            .ok()
            .map(|position| self.ends[position].end)
    }

    fn statement(&mut self, statement: &Statement, parent: Option<u32>) {
        let kind = match statement {
            Statement::Function { .. } => RegionKind::Function,
            Statement::ForRange { .. } | Statement::ForIn { .. } | Statement::WhileLoop { .. } => {
                RegionKind::Loop
            }
            Statement::Conditional { .. } | Statement::Switch { .. } => RegionKind::Branch,
            // Declaring a type or an enum runs nothing, and `indicator()`, `strategy()` and
            // `library()` only configure the script; the interpreter walks past them each bar.
            Statement::TypeDeclaration { .. } | Statement::EnumDeclaration { .. } => return,
            Statement::Expression(Expression::Call { name, .. })
                if matches!(name.as_str(), "indicator" | "strategy" | "library") =>
            {
                return;
            }
            _ => RegionKind::Statement,
        };
        let range = statement.range();
        let end = self
            .end_of(range.start.offset)
            .filter(|end| end.offset >= range.start.offset)
            .unwrap_or(range.end);
        let name = match statement {
            Statement::Function { name, .. } => Some(name.clone()),
            _ => None,
        };
        let index = self.regions.len() as u32;
        if let Some(parent) = parent {
            self.regions[parent as usize].has_children = true;
        }
        self.regions.push(Region {
            kind,
            offset: range.start.offset,
            start: range.start,
            end,
            parent,
            blocks: Vec::new(),
            name,
            has_children: false,
        });
        // Statements a value block (`x = if …`) holds run while this one does.
        for expression in traversal::expressions(statement) {
            self.expression(expression, index);
        }
        for body in traversal::nested_blocks(statement) {
            let first = self.block(body, Some(index));
            self.regions[index as usize].blocks.push(first);
        }
    }

    fn expression(&mut self, expression: &Expression, parent: u32) {
        if let Expression::Statement(inner, _) = expression {
            self.statement(inner, Some(parent));
        }
        for child in traversal::children(expression) {
            self.expression(child, parent);
        }
    }
}
