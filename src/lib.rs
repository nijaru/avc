//! Canonical AVC v1 object codec. This unreleased format is not yet qualified.

pub mod codec;
pub mod engine;
pub mod git;
pub mod materialize;
pub mod repository;
pub mod text;

pub use codec::{
    BLOB_KIND, CONFLICT_KIND, Conflict, DecodeLimits, Entry, Error, Id, MAP_BRANCH_KIND,
    MAP_LEAF_KIND, Node, Object, REVISION_KIND, Result, Revision, validate_name, validate_path,
};
