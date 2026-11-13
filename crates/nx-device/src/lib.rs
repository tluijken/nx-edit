//! The pedal as nx-edit talks to it: reading its presets into `.hlx` form, and a live connection
//! that edits them. Shared by the editor and by anything else that drives the pedal.

pub mod device;
pub mod live;
