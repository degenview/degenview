//! gettext's `Plural-Forms` header: which `msgstr[i]` a count takes.
//!
//! A catalog says it itself, `nplurals=3; plural=(n%10==1 && n%100!=11 ? 0 : …);`, in the
//! expression language GNU gettext defines (C's operators on an unsigned `n`). Translation tools
//! write and check that header for each language, so the app follows it and keeps no table of
//! languages' rules of its own. The tests check every formula it ships against CLDR.

/// How many forms a language has and which one `n` takes.
#[derive(Clone, Debug, PartialEq)]
pub struct PluralForms {
    count: usize,
    expr: Expr,
}

#[derive(Debug, PartialEq, thiserror::Error)]
#[error("the plural forms {text:?} are not valid: {reason}")]
pub struct PluralFormsError {
    text: String,
    reason: &'static str,
}

#[derive(Clone, Debug, PartialEq)]
enum Expr {
    N,
    Number(u64),
    Not(Box<Expr>),
    Binary(Op, Box<Expr>, Box<Expr>),
    Conditional(Box<Expr>, Box<Expr>, Box<Expr>),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Op {
    Or,
    And,
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
}

impl PluralForms {
    /// What gettext assumes of a catalog that has no `Plural-Forms` header, and what English and
    /// every language that only tells one from many says: `nplurals=2; plural=n != 1;`.
    pub fn english() -> Self {
        Self::parse(2, "n != 1").expect("the English formula is valid")
    }

    /// The forms a `Plural-Forms` header declares: `nplurals=3; plural=EXPR;`.
    pub fn from_header(header: &str) -> Result<Self, PluralFormsError> {
        let fail = |reason| PluralFormsError {
            text: header.to_owned(),
            reason,
        };
        let (mut count, mut formula) = (None, None);
        for part in header.split(';') {
            // The formula has `=` of its own (`==`), so only the first one separates the key.
            let Some((key, value)) = part.split_once('=') else {
                continue;
            };
            match key.trim() {
                "nplurals" => {
                    let value = value.trim().parse::<usize>();
                    count = Some(value.map_err(|_| fail("nplurals is not a number"))?);
                }
                "plural" => formula = Some(value.trim()),
                _ => {}
            }
        }
        let count = count.ok_or_else(|| fail("nplurals is missing"))?;
        let formula = formula.ok_or_else(|| fail("plural is missing"))?;
        Self::parse(count, formula)
    }

    /// The forms of a language with `count` of them and `formula` for the one a count takes.
    pub fn parse(count: usize, formula: &str) -> Result<Self, PluralFormsError> {
        let fail = |reason| PluralFormsError {
            text: formula.to_owned(),
            reason,
        };
        if count == 0 {
            return Err(fail("nplurals is 0"));
        }
        let mut parser = Parser {
            text: formula.as_bytes(),
            at: 0,
        };
        let expr = parser.conditional().map_err(fail)?;
        parser.skip_space();
        if parser.at != parser.text.len() {
            return Err(fail("unexpected text after the formula"));
        }
        Ok(Self { count, expr })
    }

    /// How many forms the language has (`nplurals`).
    pub fn count(&self) -> usize {
        self.count
    }

    /// The index of the form `n` takes, never past the last form.
    pub fn index(&self, n: u64) -> usize {
        let index = self.expr.eval(n);
        usize::try_from(index).map_or(self.count - 1, |index| index.min(self.count - 1))
    }
}

impl Expr {
    fn eval(&self, n: u64) -> u64 {
        match self {
            Expr::N => n,
            Expr::Number(value) => *value,
            Expr::Not(inner) => u64::from(inner.eval(n) == 0),
            Expr::Conditional(test, yes, no) => {
                if test.eval(n) != 0 {
                    yes.eval(n)
                } else {
                    no.eval(n)
                }
            }
            Expr::Binary(op, left, right) => {
                let (left, right) = (left.eval(n), right.eval(n));
                match op {
                    Op::Or => u64::from(left != 0 || right != 0),
                    Op::And => u64::from(left != 0 && right != 0),
                    Op::Equal => u64::from(left == right),
                    Op::NotEqual => u64::from(left != right),
                    Op::Less => u64::from(left < right),
                    Op::LessEqual => u64::from(left <= right),
                    Op::Greater => u64::from(left > right),
                    Op::GreaterEqual => u64::from(left >= right),
                    Op::Add => left.wrapping_add(right),
                    Op::Subtract => left.wrapping_sub(right),
                    Op::Multiply => left.wrapping_mul(right),
                    // A formula that divides by zero has no answer; gettext uses the first form.
                    Op::Divide => left.checked_div(right).unwrap_or(0),
                    Op::Remainder => left.checked_rem(right).unwrap_or(0),
                }
            }
        }
    }
}

/// Recursive descent over C's precedence, loosest first: `?:`, `||`, `&&`, equality, comparison,
/// `+ -`, `* / %`, `!`.
struct Parser<'a> {
    text: &'a [u8],
    at: usize,
}

type Parsed = Result<Expr, &'static str>;

impl Parser<'_> {
    fn skip_space(&mut self) {
        while self.text.get(self.at).is_some_and(u8::is_ascii_whitespace) {
            self.at += 1;
        }
    }

    /// Consumes `token` when it is next.
    fn eat(&mut self, token: &str) -> bool {
        self.skip_space();
        if self.text[self.at..].starts_with(token.as_bytes()) {
            self.at += token.len();
            true
        } else {
            false
        }
    }

    /// Consumes the first of `tokens` that is next, longest first by the caller's order.
    fn eat_op(&mut self, tokens: &[(&str, Op)]) -> Option<Op> {
        tokens
            .iter()
            .find_map(|(token, op)| self.eat(token).then_some(*op))
    }

    fn conditional(&mut self) -> Parsed {
        let test = self.binary(0)?;
        if self.eat("?") {
            let yes = self.conditional()?;
            if !self.eat(":") {
                return Err("`?` without `:`");
            }
            let no = self.conditional()?;
            return Ok(Expr::Conditional(
                Box::new(test),
                Box::new(yes),
                Box::new(no),
            ));
        }
        Ok(test)
    }

    fn binary(&mut self, level: usize) -> Parsed {
        const LEVELS: [&[(&str, Op)]; 6] = [
            &[("||", Op::Or)],
            &[("&&", Op::And)],
            &[("==", Op::Equal), ("!=", Op::NotEqual)],
            &[
                ("<=", Op::LessEqual),
                (">=", Op::GreaterEqual),
                ("<", Op::Less),
                (">", Op::Greater),
            ],
            &[("+", Op::Add), ("-", Op::Subtract)],
            &[("*", Op::Multiply), ("/", Op::Divide), ("%", Op::Remainder)],
        ];
        let Some(tokens) = LEVELS.get(level) else {
            return self.unary();
        };
        let mut left = self.binary(level + 1)?;
        while let Some(op) = self.eat_op(tokens) {
            let right = self.binary(level + 1)?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn unary(&mut self) -> Parsed {
        self.skip_space();
        // `!=` is not a negation.
        if self.text[self.at..].starts_with(b"!") && !self.text[self.at..].starts_with(b"!=") {
            self.at += 1;
            return Ok(Expr::Not(Box::new(self.unary()?)));
        }
        if self.eat("(") {
            let inner = self.conditional()?;
            return if self.eat(")") {
                Ok(inner)
            } else {
                Err("a `(` is not closed")
            };
        }
        if self.text.get(self.at) == Some(&b'n') {
            self.at += 1;
            return Ok(Expr::N);
        }
        let start = self.at;
        while self.text.get(self.at).is_some_and(u8::is_ascii_digit) {
            self.at += 1;
        }
        if start == self.at {
            return Err("expected `n`, a number or `(`");
        }
        std::str::from_utf8(&self.text[start..self.at])
            .ok()
            .and_then(|digits| digits.parse().ok())
            .map(Expr::Number)
            .ok_or("a number is too large")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn forms(count: usize, formula: &str) -> PluralForms {
        PluralForms::parse(count, formula).expect("valid")
    }

    #[test]
    fn english_tells_one_from_everything_else() {
        let english = PluralForms::english();
        assert_eq!(english.count(), 2);
        assert_eq!(english.index(1), 0);
        assert_eq!(english.index(0), 1);
        assert_eq!(english.index(2), 1);
    }

    #[test]
    fn russian_has_three_forms_by_the_last_digits() {
        let russian = forms(
            3,
            "(n%10==1 && n%100!=11 ? 0 : n%10>=2 && n%10<=4 && (n%100<10 || n%100>=20) ? 1 : 2)",
        );
        let indexes: Vec<usize> = [1, 2, 4, 5, 11, 12, 21, 22, 25, 101, 111]
            .into_iter()
            .map(|n| russian.index(n))
            .collect();
        assert_eq!(indexes, [0, 1, 1, 2, 2, 2, 0, 1, 2, 0, 2]);
    }

    #[test]
    fn a_language_with_one_form_is_always_the_first() {
        let japanese = forms(1, "0");
        assert_eq!(japanese.index(0), 0);
        assert_eq!(japanese.index(1_000), 0);
    }

    #[test]
    fn arabic_has_six_forms() {
        let arabic = forms(
            6,
            "n==0 ? 0 : n==1 ? 1 : n==2 ? 2 : n%100>=3 && n%100<=10 ? 3 : n%100>=11 ? 4 : 5",
        );
        let indexes: Vec<usize> = [0, 1, 2, 3, 11, 100, 103, 111]
            .into_iter()
            .map(|n| arabic.index(n))
            .collect();
        assert_eq!(indexes, [0, 1, 2, 3, 4, 5, 3, 4]);
    }

    #[test]
    fn negation_and_not_equal_are_told_apart() {
        assert_eq!(forms(2, "!(n==1)").index(2), 1);
        assert_eq!(forms(2, "!n").index(0), 1);
        assert_eq!(forms(2, "n!=1").index(1), 0);
    }

    #[test]
    fn an_index_past_the_last_form_is_the_last_form() {
        assert_eq!(forms(2, "5").index(1), 1);
    }

    #[test]
    fn arithmetic_follows_c_precedence_and_division_by_zero_is_the_first_form() {
        assert_eq!(forms(9, "1 + 2 * 3").index(0), 7);
        assert_eq!(forms(9, "(1 + 2) * 3").index(0), 8);
        assert_eq!(forms(9, "n / 0").index(5), 0);
        assert_eq!(forms(9, "n % 0").index(5), 0);
    }

    #[test]
    fn a_header_gives_the_count_and_the_formula() {
        let russian = PluralForms::from_header(
            "nplurals=3; plural=(n%10==1 && n%100!=11 ? 0 : n%10>=2 && n%10<=4 && (n%100<10 || n%100>=20) ? 1 : 2);",
        )
        .expect("valid");
        assert_eq!(russian.count(), 3);
        assert_eq!(russian.index(22), 1);
        // Spacing, a missing final semicolon and a formula that is only a comparison.
        let german = PluralForms::from_header("nplurals=2;plural=n!=1").expect("valid");
        assert_eq!(
            (german.count(), german.index(1), german.index(2)),
            (2, 0, 1)
        );
        let french =
            PluralForms::from_header("  nplurals = 2 ;  plural = n > 1 ; ").expect("valid");
        assert_eq!(
            (french.index(0), french.index(1), french.index(2)),
            (0, 0, 1)
        );
    }

    #[test]
    fn a_header_that_does_not_say_both_is_refused() {
        for bad in [
            "",
            "nplurals=2",
            "plural=n != 1",
            "nplurals=two; plural=0;",
            "nplurals=0; plural=0;",
        ] {
            assert!(PluralForms::from_header(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_formula_that_is_not_valid_is_refused() {
        for bad in [
            "",
            "n ==",
            "(n",
            "n ? 1",
            "n 1",
            "n && ",
            "x",
            "99999999999999999999999",
        ] {
            assert!(PluralForms::parse(2, bad).is_err(), "{bad:?}");
        }
        assert!(PluralForms::parse(0, "0").is_err());
    }
}
