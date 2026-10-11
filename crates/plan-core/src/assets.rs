//! Furniture asset library types (shared between backend and frontend).

use serde::{Deserialize, Serialize};

use crate::model::{Id, LightKind};
use crate::primitives::PrimitiveShape;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum AssetKind {
    Ikea,
    Upload,
    Primitive,
    /// Generated from a photo (TRELLIS.2).
    Photo,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct IkeaMeta {
    pub url: String,
    pub article_no: String,
    pub price: Option<f64>,
    pub currency: Option<String>,
    pub image_url: Option<String>,
}

/// Light that comes with an asset (e.g. a floor lamp).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct AssetLight {
    pub kind: LightKind,
    /// Offset from the footprint centre bottom, in the asset's local frame (mm, z up), at default dims.
    pub offset_mm: [i32; 3],
    pub lumen: u32,
    pub color_temp_k: u16,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Asset {
    pub id: Id,
    pub kind: AssetKind,
    pub name: String,
    /// Real-world dimensions w × d × h (mm).
    pub dims_mm: [u32; 3],
    /// URL of the GLB (Ikea/Upload). The model is scaled to the instance dims.
    pub model_url: Option<String>,
    pub thumbnail_url: Option<String>,
    pub primitive: Option<PrimitiveShape>,
    pub ikea: Option<IkeaMeta>,
    pub light: Option<AssetLight>,
    pub warnings: Vec<String>,
    pub created_at: String,
    /// Occupied parts of the box (see `FurnitureInstance::footprint`); empty = whole box, None = not computed yet.
    #[serde(default)]
    pub footprint: Option<Vec<[f32; 4]>>,
}
