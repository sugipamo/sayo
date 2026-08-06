use crate::error::{Error, Result};
use crate::world::MemoryWorld;

const EMBEDDED_FILES: &[(&str, &[u8])] = &[
    ("VERSION", include_bytes!("../standard_world/VERSION")),
    (
        "For/.any/Each/.keep",
        include_bytes!("../standard_world/For/.any/Each/.keep"),
    ),
    (
        "Each/.any/EachApply/.keep",
        include_bytes!("../standard_world/Each/.any/EachApply/.keep"),
    ),
    (
        "EachApply/.tick/EachEmit/main.sayo",
        include_bytes!("../standard_world/EachApply/.tick/EachEmit/main.sayo"),
    ),
    (
        "EachEmit/.tick/Each/main.sayo",
        include_bytes!("../standard_world/EachEmit/.tick/Each/main.sayo"),
    ),
    (
        "Filter/.any/Sieve/.keep",
        include_bytes!("../standard_world/Filter/.any/Sieve/.keep"),
    ),
    (
        "Sieve/.any/SieveApply/.keep",
        include_bytes!("../standard_world/Sieve/.any/SieveApply/.keep"),
    ),
    (
        "SieveApply/.tick/.any/main.sayo",
        include_bytes!("../standard_world/SieveApply/.tick/.any/main.sayo"),
    ),
    (
        "SieveChoose/Keep/Sieve/main.sayo",
        include_bytes!("../standard_world/SieveChoose/Keep/Sieve/main.sayo"),
    ),
    (
        "SieveChoose/Drop/Sieve/main.sayo",
        include_bytes!("../standard_world/SieveChoose/Drop/Sieve/main.sayo"),
    ),
    (
        "SieveChoose/.any/Trap/main.sayo",
        include_bytes!("../standard_world/SieveChoose/.any/Trap/main.sayo"),
    ),
    (
        "SieveEmit/.tick/Sieve/main.sayo",
        include_bytes!("../standard_world/SieveEmit/.tick/Sieve/main.sayo"),
    ),
    (
        "SieveDiscard/.tick/Sieve/main.sayo",
        include_bytes!("../standard_world/SieveDiscard/.tick/Sieve/main.sayo"),
    ),
    (
        "Fold/.any/FoldInit/.keep",
        include_bytes!("../standard_world/Fold/.any/FoldInit/.keep"),
    ),
    (
        "FoldInit/.any/FoldAccum/.keep",
        include_bytes!("../standard_world/FoldInit/.any/FoldAccum/.keep"),
    ),
    (
        "FoldInit/.end/Trap/main.sayo",
        include_bytes!("../standard_world/FoldInit/.end/Trap/main.sayo"),
    ),
    (
        "FoldAccum/.any/FoldApply/.keep",
        include_bytes!("../standard_world/FoldAccum/.any/FoldApply/.keep"),
    ),
    (
        "FoldAccum/.end/FoldClose/main.sayo",
        include_bytes!("../standard_world/FoldAccum/.end/FoldClose/main.sayo"),
    ),
    (
        "FoldApply/.tick/FoldNext/main.sayo",
        include_bytes!("../standard_world/FoldApply/.tick/FoldNext/main.sayo"),
    ),
    (
        "FoldNext/.tick/FoldAccum/main.sayo",
        include_bytes!("../standard_world/FoldNext/.tick/FoldAccum/main.sayo"),
    ),
    (
        "FoldClose/.tick/Vanish/main.sayo",
        include_bytes!("../standard_world/FoldClose/.tick/Vanish/main.sayo"),
    ),
    (
        "Reduce/.any/ReduceInit/.keep",
        include_bytes!("../standard_world/Reduce/.any/ReduceInit/.keep"),
    ),
    (
        "ReduceInit/.any/ReduceAccum/.keep",
        include_bytes!("../standard_world/ReduceInit/.any/ReduceAccum/.keep"),
    ),
    (
        "ReduceInit/.end/Trap/main.sayo",
        include_bytes!("../standard_world/ReduceInit/.end/Trap/main.sayo"),
    ),
    (
        "ReduceAccum/.any/ReduceApply/.keep",
        include_bytes!("../standard_world/ReduceAccum/.any/ReduceApply/.keep"),
    ),
    (
        "ReduceAccum/.end/ReduceClose/main.sayo",
        include_bytes!("../standard_world/ReduceAccum/.end/ReduceClose/main.sayo"),
    ),
    (
        "ReduceApply/.tick/ReduceNext/main.sayo",
        include_bytes!("../standard_world/ReduceApply/.tick/ReduceNext/main.sayo"),
    ),
    (
        "ReduceNext/.tick/ReduceAccum/main.sayo",
        include_bytes!("../standard_world/ReduceNext/.tick/ReduceAccum/main.sayo"),
    ),
    (
        "ReduceClose/.tick/Vanish/main.sayo",
        include_bytes!("../standard_world/ReduceClose/.tick/Vanish/main.sayo"),
    ),
    (
        "Duplicate/Int/EmitTwice/.keep",
        include_bytes!("../standard_world/Duplicate/Int/EmitTwice/.keep"),
    ),
    (
        "EqualTo/.any/EqualToExpected/.keep",
        include_bytes!("../standard_world/EqualTo/.any/EqualToExpected/.keep"),
    ),
    (
        "EqualToExpected/.any/EqualToApply/.keep",
        include_bytes!("../standard_world/EqualToExpected/.any/EqualToApply/.keep"),
    ),
    (
        "EqualToApply/.tick/.any/main.sayo",
        include_bytes!("../standard_world/EqualToApply/.tick/.any/main.sayo"),
    ),
    (
        "EmitTwice/.tick/EmitPair/main.sayo",
        include_bytes!("../standard_world/EmitTwice/.tick/EmitPair/main.sayo"),
    ),
    (
        "EmitPair/.tick/EmitOnce/main.sayo",
        include_bytes!("../standard_world/EmitPair/.tick/EmitOnce/main.sayo"),
    ),
    (
        "EmitOnce/.tick/Duplicate/main.sayo",
        include_bytes!("../standard_world/EmitOnce/.tick/Duplicate/main.sayo"),
    ),
];

/// Build the standard World without touching a host filesystem. This is the
/// representation used by both the native Runtime and browser/Wasm adapter.
pub fn embedded_standard_world() -> Result<MemoryWorld> {
    let mut world = MemoryWorld::new();
    for &(relative, contents) in EMBEDDED_FILES {
        let contents = std::str::from_utf8(contents).map_err(|error| {
            Error::World(format!(
                "embedded standard World file {relative} is not UTF-8: {error}"
            ))
        })?;
        world.insert_file(relative, contents)?;
    }
    Ok(world)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_the_embedded_world_without_filesystem_extraction() {
        use crate::world::{WorldPath, WorldView};

        let world = embedded_standard_world().unwrap();
        assert_eq!(
            world
                .read(&WorldPath::new("EachApply/.tick/EachEmit/main.sayo").unwrap())
                .unwrap()
                .as_deref(),
            Some("%0 (%0 %1)\n")
        );
        assert!(
            world
                .is_dir(&WorldPath::new("FoldClose/.tick/Vanish").unwrap())
                .unwrap()
        );
    }
}
