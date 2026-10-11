//! Result types of the floor plan import (AI proposal).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::model::{Id, RefImage};
use crate::ops::Op;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ReferenceSuggestion {
    /// Two plan points (mm) and the length the plan says lies between them.
    pub a: [f64; 2],
    pub b: [f64; 2],
    pub length_mm: f64,
    pub label: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct FloorplanProposal {
    /// Ops to apply to the (empty or current) floor.
    pub ops: Vec<Op>,
    /// Confidence 0..1 per element id (walls, openings, rooms).
    pub confidence: HashMap<Id, f32>,
    pub reference: Option<ReferenceSuggestion>,
    pub north_angle_deg: Option<f32>,
    pub image: RefImage,
    pub mm_per_px: f64,
    /// Human-readable notes (German), e.g. scale outliers.
    pub notes: Vec<String>,
    /// Whether the LLM stage contributed (false = CV only).
    pub used_llm: bool,
}
