//! Independent embroidery geometry, generation and machine-format code.
//! Coordinates are millimetres, x right, y down. DST conversion flips y.
pub mod autodigitize;
pub mod axis_edit;
pub mod centerline;
pub mod centerline_workflow;
pub mod color_selection;
pub mod commands;
pub mod dsb;
pub mod dst;
pub mod engine;
pub mod import;
pub mod model;
pub mod pdf_import;
pub mod persistence;
pub mod quality;
pub mod region_edit;
pub mod satin_edit;
pub mod thinning;
pub use model::*;
