use std::any::{type_name, TypeId};
use std::collections::{btree_map::Entry, BTreeMap};

use ts_rs::{Config as TsConfig, TypeVisitor, TS};

use crate::{
    download::ModrinthInstallPhase,
    error::AppErrorWire,
    wire::{self, *},
    *,
};

#[derive(Default)]
pub struct WireExports {
    types: BTreeMap<String, Declaration>,
}

struct Declaration {
    rust_id: TypeId,
    rust_name: &'static str,
    typescript: String,
}

fn config() -> TsConfig {
    TsConfig::new().with_large_int("number")
}

impl TypeVisitor for WireExports {
    fn visit<T: TS + 'static + ?Sized>(&mut self) {
        if T::output_path().is_none() {
            T::visit_generics(self);
            T::visit_dependencies(self);
            return;
        }

        let config = config();
        let name = T::ident(&config);
        let rust_id = TypeId::of::<T>();
        let rust_name = type_name::<T>();
        match self.types.entry(name) {
            Entry::Occupied(entry) => {
                assert!(
                    entry.get().rust_id == rust_id,
                    "duplicate TypeScript name `{}` for `{}` and `{rust_name}`",
                    entry.key(),
                    entry.get().rust_name,
                );
            }
            Entry::Vacant(entry) => {
                let declaration = T::decl(&config)
                    .lines()
                    .map(str::trim_end)
                    .collect::<Vec<_>>()
                    .join("\n");
                entry.insert(Declaration {
                    rust_id,
                    rust_name,
                    typescript: format!("export {declaration}\n"),
                });
                T::visit_generics(self);
                T::visit_dependencies(self);
            }
        }
    }
}

impl WireExports {
    pub fn collect() -> Self {
        let mut exports = Self::default();
        macro_rules! roots {
            ($($ty:ty),* $(,)?) => { $(exports.visit::<$ty>();)* };
        }
        roots!(
            AppErrorWire,
            GameDirState,
            GameGuard,
            PackListing,
            ImportChoice,
            ImportOutcome,
            ResourceStack,
            StackSnapshot,
            BackupInfo,
            World,
            DatapackListing,
            wire::DatapackDeleteOutcome,
            DatapackImportOutcome,
            DataSnapshot,
            LevelBackupInfo,
            SearchPage,
            ModrinthProject,
            ModrinthVersion,
            ModrinthTargets,
            ModrinthInstallOutcome,
            ModrinthInstallPhase,
            ProvenanceEntry,
            SearchKind,
            SearchSort,
        );
        exports
    }

    pub fn declarations(&self) -> impl Iterator<Item = &str> {
        self.types
            .values()
            .map(|declaration| declaration.typescript.as_str())
    }

    pub fn assert_exported<T: TS + 'static>(&self, command: &str) {
        let mut signature = Self::default();
        signature.visit::<T>();
        for (name, declaration) in signature.types {
            assert!(
                self.types
                    .get(&name)
                    .is_some_and(|export| export.rust_id == declaration.rust_id),
                "{command}: Rust type `{}` is missing from the generated wire exports",
                declaration.rust_name,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(TS)]
    #[ts(rename = "SameName")]
    struct First;

    #[derive(TS)]
    #[ts(rename = "SameName")]
    struct Second;

    #[test]
    #[should_panic(expected = "duplicate TypeScript name `SameName`")]
    fn rejects_a_duplicate_export_name_even_when_the_declarations_match() {
        let mut exports = WireExports::default();
        exports.visit::<First>();
        exports.visit::<Second>();
    }
}
