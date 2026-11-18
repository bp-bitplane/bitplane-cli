//! The wire types as TypeScript, for clients that should not hand-copy them.
//!
//! `bp-contract.ts` at this crate's root is the committed output. The test
//! below fails when it is stale; `BP_CONTRACT_WRITE=1` rewrites it.

use specta::Types;
use specta::datatype::{DataType, Fields};
use specta_typescript::Typescript;

use crate::error::{EngineError, ErrorEnvelope};
use crate::wire::{ProgressEvent, Request, Response, VersionInfo};

/// Where the committed copy lives, relative to this crate.
pub const FILE_NAME: &str = "bp-contract.ts";

/// Every type a client of `bp` reads or writes, and what they reach.
pub fn typescript() -> Result<String, String> {
    let mut types = Types::default()
        .register::<Request>()
        .register::<Response>()
        .register::<EngineError>()
        .register::<ErrorEnvelope>()
        .register::<ProgressEvent>()
        .register::<VersionInfo>();
    types.iter_mut(|named| {
        if let Some(ty) = named.ty.as_mut() {
            optional_where_omitted(ty);
        }
    });

    Typescript::default()
        .export(&types, specta_serde::Format)
        .map_err(|err| err.to_string())
}

/// A field serde leaves out when empty is an optional field to a reader.
///
/// specta-serde refuses `skip_serializing_if` in unified mode, and its phased
/// mode renders every enum holding such a type as externally tagged, which the
/// wire is not. Requests and responses share one shape, so one type is right.
fn optional_where_omitted(ty: &mut DataType) {
    let fields = match ty {
        DataType::Struct(structure) => vec![&mut structure.fields],
        DataType::Enum(enumeration) => enumeration
            .variants
            .iter_mut()
            .map(|(_, variant)| &mut variant.fields)
            .collect(),
        DataType::List(list) => return optional_where_omitted(&mut list.ty),
        DataType::Nullable(inner) => return optional_where_omitted(inner),
        DataType::Intersection(parts) => return parts.iter_mut().for_each(optional_where_omitted),
        DataType::Tuple(tuple) => {
            return tuple.elements.iter_mut().for_each(optional_where_omitted);
        }
        _ => return,
    };

    for fields in fields {
        let fields: Vec<&mut specta::datatype::Field> = match fields {
            Fields::Named(named) => named.fields.iter_mut().map(|(_, field)| field).collect(),
            Fields::Unnamed(unnamed) => unnamed.fields.iter_mut().collect(),
            Fields::Unit => Vec::new(),
        };
        for field in fields {
            if field.attributes.remove(SKIP_SERIALIZING_IF) {
                field.optional = true;
            }
            if let Some(ty) = field.ty.as_mut() {
                optional_where_omitted(ty);
            }
        }
    }
}

/// The key specta-serde stores `#[serde(skip_serializing_if)]` under.
const SKIP_SERIALIZING_IF: &str = "serde:field:skip_serializing_if";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_committed_typescript_is_current() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(FILE_NAME);
        let generated = typescript().expect("the wire types export");

        if std::env::var_os("BP_CONTRACT_WRITE").is_some() {
            std::fs::write(&path, &generated).expect("write bp-contract.ts");
            return;
        }

        let committed = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            committed == generated,
            "{FILE_NAME} is stale. Regenerate it with\n  \
             BP_CONTRACT_WRITE=1 cargo test -p bitplane-core --features typescript contract"
        );
    }

    #[test]
    fn a_string_newtype_is_a_string_and_a_counter_is_a_number() {
        let generated = typescript().unwrap();

        for line in [
            "export type MemberRef = string;",
            "export type ProjectName = string;",
            "export type PlaneId = string;",
            "export type WorktreePath = string;",
            "\tupdated: number,",
            "\tduration_ms: number,",
            "\tcode: number,",
        ] {
            assert!(generated.contains(line), "missing {line:?}");
        }
        assert!(!generated.contains("bigint"));
        assert!(
            !generated.contains("number | null,\n"),
            "a counter is never null"
        );
    }

    #[test]
    fn a_tagged_enum_keeps_its_tag() {
        let generated = typescript().unwrap();

        for tag in [
            "action: \"plane_create\"",
            "outcome: \"ok\"",
            "error: \"refused\"",
            "event: \"waiting\"",
        ] {
            assert!(generated.contains(tag), "missing {tag:?}");
        }
    }
}
