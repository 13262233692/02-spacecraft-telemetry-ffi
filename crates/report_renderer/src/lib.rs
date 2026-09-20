//! `report_renderer`: ANSI colored terminal reports for TM frame tooling.

pub mod color;
pub mod diff;
pub mod infer;
pub mod parse;

pub use color::set_color;
pub use diff::render_diff;
pub use infer::render_reports as render_inference_reports;
pub use parse::render_frames as render_parse_report;
