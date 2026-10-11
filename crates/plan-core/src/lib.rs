//! `plan-core`: the parametric apartment model, operations, geometry and validation.
//! Runs natively in the backend and as WebAssembly in the browser.

pub mod assets;
pub mod commands;
pub mod furniture;
pub mod geom;
pub mod import;
pub mod mesh;
pub mod model;
pub mod ops;
pub mod primitives;
pub mod rooms;
pub mod sun;
pub mod validate;
pub mod walls;

pub use model::*;
pub use ops::{apply, apply_batch, Op, OpError};
