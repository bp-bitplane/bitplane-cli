//! The wire types, as TypeScript (`bp-contract.ts`).
//!
//! Generated rather than copied: a client that hand-copies `Request` has
//! already drifted from it by the next release. Behind the `ts` feature,
//! because nothing a `bp` binary does needs a generator, and with the same
//! pinned specta bitplane-cc uses, so the two read each other's output.
//!
//! Generating changes no wire byte. The derives only describe what serde
//! already writes; where a type serialises as something other than its Rust
//! shape — a newtype as its string, a count as a plain number — it says so on
//! the type.

use std::borrow::Cow;

use specta::datatype::{DataType, Fields};
use specta::{FormatError, Types};
use specta_typescript::Typescript;

use crate::error::ErrorEnvelope;
use crate::wire::{CONTRACT_VERSION, ProgressEvent, Request, Response};

/// Where the generated file is committed, relative to this crate.
pub const PATH: &str = "bp-contract.ts";

/// `bp-contract.ts`, as it should read for the types in this build.
///
/// The roots are what a client sends or reads: a request, the response or
/// envelope that answers it, and the progress written while it runs.
/// Everything they reference comes along.
pub fn typescript() -> Result<String, specta_typescript::Error> {
    let types = Types::default()
        .register::<Request>()
        .register::<Response>()
        .register::<ErrorEnvelope>()
        .register::<ProgressEvent>();

    let body = Typescript::default().header(HEADER).export(&types, Wire)?;

    Ok(format!(
        "{body}\n/** Which revision of `Request` and `Response` this file describes. `bp --version --json` reports it as `contract`. */\nexport const CONTRACT_VERSION = {CONTRACT_VERSION};\n"
    ))
}

const HEADER: &str = "// Generated from bitplane-core's wire types. Do not edit by hand:\n\
                      // run `cargo test -p bitplane-core --features ts --test contract` with\n\
                      // BITPLANE_BLESS=1 set, and commit the result.";

/// serde's own reading of the types, plus the one thing its unified mode will
/// not say.
///
/// A field under `skip_serializing_if` is left out of what `bp` writes and
/// defaulted on what it reads, which a single TypeScript shape says exactly:
/// the field is optional. `specta_serde::Format` refuses to say it, and
/// `PhasesFormat` would split every type that reaches such a field into a
/// `_Serialize` and a `_Deserialize` copy — `Response` included — to describe
/// one boolean. So the field is marked optional here first.
struct Wire;

impl specta::Format for Wire {
    fn map_types(&'_ self, types: &Types) -> Result<Cow<'_, Types>, FormatError> {
        let mut types = types.clone();
        types.iter_mut(|named| {
            if let Some(ty) = named.ty.as_mut() {
                omittable_is_optional(ty);
            }
        });

        Ok(Cow::Owned(
            specta::Format::map_types(&specta_serde::Format, &types)?.into_owned(),
        ))
    }

    fn map_type(&'_ self, types: &Types, dt: &DataType) -> Result<Cow<'_, DataType>, FormatError> {
        let mut dt = dt.clone();
        omittable_is_optional(&mut dt);

        Ok(Cow::Owned(
            specta::Format::map_type(&specta_serde::Format, types, &dt)?.into_owned(),
        ))
    }
}

/// The key specta-serde reads `skip_serializing_if` from.
const SKIP_SERIALIZING_IF: &str = "serde:field:skip_serializing_if";

fn omittable_is_optional(ty: &mut DataType) {
    match ty {
        DataType::Struct(shape) => fields_of(&mut shape.fields),
        DataType::Enum(shape) => {
            for (_, variant) in &mut shape.variants {
                fields_of(&mut variant.fields);
            }
        }
        _ => {}
    }
}

fn fields_of(fields: &mut Fields) {
    if let Fields::Named(named) = fields {
        for (_, field) in &mut named.fields {
            if field.attributes.remove(SKIP_SERIALIZING_IF) {
                field.optional = true;
            }
        }
    }
}
