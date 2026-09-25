//! The rendering macros, grouped by what they render.
//!
//! Each macro takes the parts that differ between dialects as parameters
//! (`$quote`, `$render_expr`, `$param_cast`), so a dialect module supplies only
//! its own behaviour. Identifier parameters are substituted at the call site,
//! which is the point: `$render_expr` resolves to the calling dialect's own
//! expression renderer. A clause that binds nothing and renders no expression
//! needs neither, so it is a function in [`crate::clauses`] instead.
//!
//! Every render path consumes the AST by value, moving bound values out instead
//! of cloning them, so these macros take `&mut` and are the single source of
//! truth for the SQL. The borrowed `Dialect::render_*` entry points clone the
//! AST once and delegate here.

pub(crate) mod expressions;
pub(crate) mod select;
pub(crate) mod write;
