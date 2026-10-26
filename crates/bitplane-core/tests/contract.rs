//! `bp-contract.ts` is what the wire types generate, byte for byte, and says
//! what the wire says.
//!
//! The file is committed so a client can take it without a Rust toolchain, and
//! a committed generator output stops matching its input the moment somebody
//! edits the input alone. This is what notices.

use std::fs;
use std::path::PathBuf;

#[cfg(feature = "ts")]
use bitplane_core::contract;

#[cfg(feature = "ts")]
#[test]
fn the_committed_contract_is_what_the_wire_types_generate() {
    let generated = contract::typescript().expect("the wire types describe themselves");
    let committed_at = crate_dir().join(contract::PATH);

    if std::env::var_os("BITPLANE_BLESS").is_some() {
        fs::write(&committed_at, &generated).expect("write the contract");
        return;
    }

    let committed = fs::read_to_string(&committed_at).unwrap_or_default();
    assert!(
        committed == generated,
        "{} is stale: the wire types changed and it was not regenerated.\n\
         Run `BITPLANE_BLESS=1 cargo test -p bitplane-core --features ts --test contract` \
         and commit the result.",
        committed_at.display()
    );
}

/// A newtype serialises as its string and a count as a plain number, so that
/// is what a client has to be told — not the Rust shape behind it.
#[cfg(feature = "ts")]
#[test]
fn newtypes_are_strings_and_counts_are_numbers() {
    let generated = contract::typescript().unwrap();

    for newtype in [
        "MemberRef",
        "ProjectName",
        "WorktreePath",
        "PlaneId",
        "GitVersion",
    ] {
        assert!(
            generated.contains(&format!("export type {newtype} = string;")),
            "{newtype} is not a string:\n{generated}"
        );
    }
    for count in [
        "modified: number",
        "updated: number",
        "duration_ms: number",
        "failed: number",
    ] {
        assert!(generated.contains(count), "no `{count}`:\n{generated}");
    }
    assert!(generated.contains("export type ExitCode = number;"));
    assert!(!generated.contains("bigint"), "nothing is a bigint");
}

/// The tags are how a client tells one variant from another, so each tagged
/// enum keeps the tag serde writes.
#[cfg(feature = "ts")]
#[test]
fn tagged_enums_keep_their_tags() {
    let generated = contract::typescript().unwrap();

    for tag in [
        "action: \"plane_create\"",
        "action: \"project_list\"",
        "outcome: \"ok\"; detail: T",
        "outcome: \"failed\"; detail: EngineError",
        "error: \"refused\"",
        "event: \"waiting\"",
        "step: \"script\"; name: string; log: string",
        "plane: \"containing_path\"; path: string",
    ] {
        assert!(generated.contains(tag), "no `{tag}`:\n{generated}");
    }
    assert!(
        generated.contains("kept_directory?: boolean"),
        "a field serde leaves out when unset is optional, not required"
    );
}

/// Without the feature, a build of `bp` pulls in no generator at all.
#[test]
fn the_generator_is_optional() {
    let manifest = fs::read_to_string(crate_dir().join("Cargo.toml")).unwrap();

    for line in manifest.lines().filter(|line| line.starts_with("specta")) {
        assert!(line.contains("optional = true"), "not optional: {line}");
    }
    assert!(
        !manifest.contains("default = ["),
        "the `ts` feature must not be on by default"
    );
}

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}
