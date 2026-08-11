//! Apply semantic TOML changes without rewriting user formatting and comments.

use toml::{Table as SemanticTable, Value as SemanticValue};
use toml_edit::{Array, ArrayOfTables, DocumentMut, InlineTable, Item, Table, Value as EditValue};

/// Merge one updated value tree into the original format-preserving document.
pub(crate) fn merge_update(
    original: &str,
    desired: &str,
    before: &SemanticTable,
    after: &SemanticTable,
) -> Result<String, String> {
    let mut original = original
        .parse::<DocumentMut>()
        .map_err(|error| error.to_string())?;
    let desired = desired
        .parse::<DocumentMut>()
        .map_err(|error| error.to_string())?;
    merge_table(original.as_table_mut(), desired.as_table(), before, after)?;
    Ok(original.to_string())
}

fn merge_table(
    original: &mut Table,
    desired: &Table,
    before: &SemanticTable,
    after: &SemanticTable,
) -> Result<(), String> {
    let removed = before
        .keys()
        .filter(|key| !after.contains_key(*key))
        .cloned()
        .collect::<Vec<_>>();
    for key in removed {
        original.remove(&key);
    }
    for (key, after_value) in after {
        if before.get(key) == Some(after_value) {
            continue;
        }
        let Some(desired_item) = desired.get(key).cloned() else {
            return Err(format!("the updated TOML document lost key {key:?}"));
        };
        if let Some(original_item) = original.get_mut(key) {
            merge_item(original_item, desired_item, before.get(key), after_value)?;
        } else {
            original.insert(key, desired_item);
        }
    }
    Ok(())
}

fn merge_item(
    original: &mut Item,
    desired: Item,
    before: Option<&SemanticValue>,
    after: &SemanticValue,
) -> Result<(), String> {
    match (original, desired, before, after) {
        (
            Item::Table(original),
            Item::Table(desired),
            Some(SemanticValue::Table(before)),
            SemanticValue::Table(after),
        ) => merge_table(original, &desired, before, after)?,
        (
            Item::Value(EditValue::InlineTable(original)),
            Item::Value(EditValue::InlineTable(desired)),
            Some(SemanticValue::Table(before)),
            SemanticValue::Table(after),
        ) => merge_inline_table(original, &desired, before, after)?,
        (
            Item::Value(EditValue::Array(original)),
            Item::Value(EditValue::Array(desired)),
            Some(SemanticValue::Array(before)),
            SemanticValue::Array(after),
        ) => merge_array(original, &desired, before, after)?,
        (
            Item::Value(EditValue::Array(original)),
            Item::ArrayOfTables(desired),
            Some(SemanticValue::Array(before)),
            SemanticValue::Array(after),
        ) => merge_array(original, &desired.into_array(), before, after)?,
        (
            Item::ArrayOfTables(original),
            Item::ArrayOfTables(desired),
            Some(SemanticValue::Array(before)),
            SemanticValue::Array(after),
        ) => merge_array_of_tables(original, &desired, before, after)?,
        (Item::Value(original), Item::Value(mut desired), _, _) => {
            *desired.decor_mut() = original.decor().clone();
            *original = desired;
        }
        (original, desired, _, _) => *original = desired,
    }
    Ok(())
}

fn merge_inline_table(
    original: &mut InlineTable,
    desired: &InlineTable,
    before: &SemanticTable,
    after: &SemanticTable,
) -> Result<(), String> {
    let removed = before
        .keys()
        .filter(|key| !after.contains_key(*key))
        .cloned()
        .collect::<Vec<_>>();
    for key in removed {
        original.remove(&key);
    }
    for (key, after_value) in after {
        if before.get(key) == Some(after_value) {
            continue;
        }
        let Some(desired_value) = desired.get(key).cloned() else {
            return Err(format!("the updated inline TOML table lost key {key:?}"));
        };
        if let Some(original_value) = original.get_mut(key) {
            merge_value(original_value, desired_value, before.get(key), after_value)?;
        } else {
            original.insert(key, desired_value);
        }
    }
    Ok(())
}

fn merge_array(
    original: &mut Array,
    desired: &Array,
    before: &[SemanticValue],
    after: &[SemanticValue],
) -> Result<(), String> {
    while original.len() > after.len() {
        original.remove(original.len() - 1);
    }
    for (index, after_value) in after.iter().enumerate() {
        if before.get(index) == Some(after_value) {
            continue;
        }
        let Some(desired_value) = desired.get(index).cloned() else {
            return Err(format!("the updated TOML array lost item {index}"));
        };
        if let Some(original_value) = original.get_mut(index) {
            merge_value(
                original_value,
                desired_value,
                before.get(index),
                after_value,
            )?;
        } else {
            original.push_formatted(desired_value);
        }
    }
    Ok(())
}

fn merge_array_of_tables(
    original: &mut ArrayOfTables,
    desired: &ArrayOfTables,
    before: &[SemanticValue],
    after: &[SemanticValue],
) -> Result<(), String> {
    while original.len() > after.len() {
        original.remove(original.len() - 1);
    }
    for (index, after_value) in after.iter().enumerate() {
        if before.get(index) == Some(after_value) {
            continue;
        }
        let Some(desired_table) = desired.get(index).cloned() else {
            return Err(format!("the updated TOML table array lost item {index}"));
        };
        if let (
            Some(original_table),
            Some(SemanticValue::Table(before)),
            SemanticValue::Table(after),
        ) = (original.get_mut(index), before.get(index), after_value)
        {
            merge_table(original_table, &desired_table, before, after)?;
        } else if index < original.len() {
            original.replace(index, desired_table);
        } else {
            original.push(desired_table);
        }
    }
    Ok(())
}

fn merge_value(
    original: &mut EditValue,
    desired: EditValue,
    before: Option<&SemanticValue>,
    after: &SemanticValue,
) -> Result<(), String> {
    match (original, desired, before, after) {
        (
            EditValue::InlineTable(original),
            EditValue::InlineTable(desired),
            Some(SemanticValue::Table(before)),
            SemanticValue::Table(after),
        ) => merge_inline_table(original, &desired, before, after)?,
        (
            EditValue::Array(original),
            EditValue::Array(desired),
            Some(SemanticValue::Array(before)),
            SemanticValue::Array(after),
        ) => merge_array(original, &desired, before, after)?,
        (original, mut desired, _, _) => {
            *desired.decor_mut() = original.decor().clone();
            *original = desired;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn semantic(text: &str) -> SemanticTable {
        text.parse().unwrap()
    }

    #[test]
    fn inline_tables_and_arrays_keep_their_existing_format() {
        let original = concat!(
            "row = { name = \"old\", keep = 1 } # row note\n",
            "items = [\"old\", \"keep\"] # array note\n",
        );
        let desired = concat!(
            "row = { name = \"new\", keep = 1 }\n",
            "items = [\"new\", \"keep\", \"added\"]\n",
        );
        let merged =
            merge_update(original, desired, &semantic(original), &semantic(desired)).unwrap();

        assert!(merged.contains("row = { name = \"new\", keep = 1 } # row note"));
        assert!(merged.contains("items = [\"new\", \"keep\", \"added\"] # array note"));
    }

    #[test]
    fn inconsistent_semantic_updates_return_a_typed_merge_reason() {
        let original = "kept = 1\n";
        let desired = "kept = 2\n";
        let mut after = semantic(desired);
        after.insert("lost".to_owned(), SemanticValue::Integer(3));
        assert!(
            merge_update(original, desired, &semantic(original), &after)
                .unwrap_err()
                .contains("lost key")
        );

        let original = "row = { kept = 1 }\n";
        let desired = "row = { kept = 2 }\n";
        let mut after = semantic(desired);
        after["row"]
            .as_table_mut()
            .unwrap()
            .insert("lost".to_owned(), SemanticValue::Integer(3));
        assert!(
            merge_update(original, desired, &semantic(original), &after)
                .unwrap_err()
                .contains("inline TOML table lost key")
        );

        let original = "items = [1]\n";
        let desired = "items = [2]\n";
        let mut after = semantic(desired);
        after["items"]
            .as_array_mut()
            .unwrap()
            .push(SemanticValue::Integer(3));
        assert!(
            merge_update(original, desired, &semantic(original), &after)
                .unwrap_err()
                .contains("array lost item 1")
        );

        let original = "[[rows]]\nname = \"old\"\n";
        let desired = "[[rows]]\nname = \"new\"\n";
        let mut after = semantic(desired);
        after["rows"]
            .as_array_mut()
            .unwrap()
            .push(SemanticValue::Table(SemanticTable::from_iter([(
                "name".to_owned(),
                SemanticValue::String("lost".to_owned()),
            )])));
        assert!(
            merge_update(original, desired, &semantic(original), &after)
                .unwrap_err()
                .contains("table array lost item 1")
        );
    }

    #[test]
    fn a_semantic_shape_change_replaces_an_existing_table_array_row() {
        let original = "[[rows]]\nname = \"old\"\n";
        let desired = "[[rows]]\nname = \"new\"\n";
        let mut before = semantic(original);
        before["rows"].as_array_mut().unwrap()[0] = SemanticValue::String("old".to_owned());

        let merged = merge_update(original, desired, &before, &semantic(desired)).unwrap();
        assert!(merged.contains("name = \"new\""));
    }

    #[test]
    fn a_semantic_shape_change_replaces_the_complete_item() {
        let original = "[row]\nname = \"old\"\n";
        let desired = "row = 2\n";

        let merged =
            merge_update(original, desired, &semantic(original), &semantic(desired)).unwrap();
        assert_eq!(semantic(&merged), semantic(desired));
    }
}
