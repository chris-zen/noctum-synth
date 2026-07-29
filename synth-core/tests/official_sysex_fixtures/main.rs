//! Official Sequential factory-bank regression tests.
//!
//! Requires local copies of the gitignored factory archives and:
//!
//! ```bash
//! RUST_MIN_STACK=16777216 cargo test -p synth-core --features official-sysex-fixtures --test official_sysex_fixtures
//! ```

mod p08;
mod rev2;
mod storage;
