//! Source text to syntax tree: tokens, the lexer, the parser, the AST, its traversal helpers and name resolution (`scopes`).

pub mod ast;
pub mod lexer;
pub mod parser;
pub mod scopes;
pub mod token;
pub mod traversal;
