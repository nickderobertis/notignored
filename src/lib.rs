//! Extract lint and type-check suppression comments from source files.
//!
//! `notignored` parses suppression ("ignore") directives — `# noqa`,
//! `// eslint-disable-next-line`, `#[allow(…)]`, … — **natively**. It never
//! shells out to the tool whose rule is being silenced, so scanning a tree costs
//! a read and a scan rather than each linter's startup.
//!
//! Every directive becomes an [`IgnoreDirective`]: which tool, which rules, the
//! stated reason, and exactly where it lives. That record and the [`Report`]
//! envelope around it are a versioned wire contract — see [`model`].
//!
//! ```
//! use notignored::{scan::ScanOptions, source::SourceFile, tools::registry_for};
//!
//! let file = SourceFile::new("src/app.py", "u = URL  # noqa: E501  # long URL\n".into());
//! let directives: Vec<_> = registry_for(&[])
//!     .iter()
//!     .filter(|parser| parser.applies_to(&file))
//!     .flat_map(|parser| parser.parse(&file))
//!     .collect();
//!
//! assert_eq!(directives[0].rules, vec!["E501"]);
//! assert_eq!(directives[0].reason.as_deref(), Some("long URL"));
//! # let _ = ScanOptions::default();
//! ```
#![warn(missing_docs)]

pub mod cli;
pub mod comments;
pub mod model;
pub mod scan;
pub mod source;
pub mod tools;

pub use model::{IgnoreDirective, Report, ReportError, Scope, Suppressed, Tool, REPORT_VERSION};
