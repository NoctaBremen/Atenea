//! Lectura de los dumps de Discogs.

pub mod model;
pub mod parser;
pub mod sample;

pub use model::{Artist, Credit, Format, LabelRef, Release, Track, Video};
pub use parser::{ParseError, ReleaseReader, open_dump};
