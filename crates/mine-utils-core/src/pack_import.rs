use crate::pack::ImportChoice;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExistingPack {
    None,
    Destination,
    Elsewhere,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportPlan {
    Copy,
    Duplicate,
    Skip,
    ReplaceDuplicate,
    ReplaceDestination,
    Collision,
}

pub fn plan_import(
    _existing: ExistingPack,
    _destination_taken: bool,
    _choice: ImportChoice,
) -> ImportPlan {
    ImportPlan::Copy
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_import_covers_every_existing_taken_choice_combination() {
        use ExistingPack::{Destination, Elsewhere, None};
        use ImportChoice::{Default, Replace, Skip};
        let cases = [
            (None, false, Default, ImportPlan::Copy),
            (None, false, Skip, ImportPlan::Copy),
            (None, false, Replace, ImportPlan::Copy),
            (None, true, Default, ImportPlan::Collision),
            (None, true, Skip, ImportPlan::Collision),
            (None, true, Replace, ImportPlan::ReplaceDestination),
            (Destination, false, Default, ImportPlan::Duplicate),
            (Destination, false, Skip, ImportPlan::Skip),
            (Destination, false, Replace, ImportPlan::ReplaceDuplicate),
            (Destination, true, Default, ImportPlan::Duplicate),
            (Destination, true, Skip, ImportPlan::Skip),
            (Destination, true, Replace, ImportPlan::ReplaceDuplicate),
            (Elsewhere, false, Default, ImportPlan::Duplicate),
            (Elsewhere, false, Skip, ImportPlan::Skip),
            (Elsewhere, false, Replace, ImportPlan::ReplaceDuplicate),
            (Elsewhere, true, Default, ImportPlan::Duplicate),
            (Elsewhere, true, Skip, ImportPlan::Skip),
            (Elsewhere, true, Replace, ImportPlan::Collision),
        ];
        for (existing, taken, choice, expected) in cases {
            assert_eq!(
                plan_import(existing, taken, choice),
                expected,
                "existing={existing:?}, taken={taken}, choice={choice:?}"
            );
        }
    }
}
