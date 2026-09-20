//! `frame_parser`: dynamically configured CCSDS TM transfer frame decoder.
//!
//! Load a [`spec::FrameSpec`] from an external JSON description, then decode
//! raw frame octets with [`decoder::decode_stream`]. [`builder`] can generate
//! spec-conformant sample frames for offline testing.

pub mod bits;
pub mod builder;
pub mod decoder;
pub mod error;
pub mod model;
pub mod spec;

pub use decoder::decode_stream;
pub use error::{ParseError, Result};
pub use model::{DecodedFrame, DecodedPacket, Field};
pub use spec::FrameSpec;
