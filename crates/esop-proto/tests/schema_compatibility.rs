use esop_proto::Message;
use prost_types::{DescriptorProto, EnumDescriptorProto, FileDescriptorSet};

fn descriptors(bytes: &[u8]) -> prost_types::FileDescriptorProto {
    let mut set = FileDescriptorSet::decode(bytes).unwrap();
    assert_eq!(
        set.file.len(),
        1,
        "extend the gate before adding schema imports"
    );
    set.file.remove(0)
}

fn current() -> prost_types::FileDescriptorProto {
    descriptors(include_bytes!(concat!(env!("OUT_DIR"), "/current.bin")))
}

fn baseline() -> prost_types::FileDescriptorProto {
    descriptors(include_bytes!(concat!(env!("OUT_DIR"), "/baseline.bin")))
}

fn enums(old: &[EnumDescriptorProto], new: &[EnumDescriptorProto]) -> Result<(), String> {
    for previous in old {
        let next = new
            .iter()
            .find(|item| item.name == previous.name)
            .ok_or_else(|| format!("missing enum {}", previous.name()))?;
        for value in &previous.value {
            if !next
                .value
                .iter()
                .any(|item| item.name == value.name && item.number == value.number)
            {
                return Err(format!("changed enum {}.{}", previous.name(), value.name()));
            }
        }
        for name in &previous.reserved_name {
            if !next.reserved_name.contains(name) {
                return Err(format!("removed enum reserved name {name}"));
            }
        }
        for range in &previous.reserved_range {
            if !next
                .reserved_range
                .iter()
                .any(|r| r.start() <= range.start() && r.end() >= range.end())
            {
                return Err(format!(
                    "removed enum reserved range in {}",
                    previous.name()
                ));
            }
        }
    }
    Ok(())
}

fn messages(old: &[DescriptorProto], new: &[DescriptorProto]) -> Result<(), String> {
    for previous in old {
        let next = new
            .iter()
            .find(|item| item.name == previous.name)
            .ok_or_else(|| format!("missing message {}", previous.name()))?;
        for field in &previous.field {
            if let Some(candidate) = next.field.iter().find(|item| item.number == field.number) {
                let old_oneof = field
                    .oneof_index
                    .and_then(|i| previous.oneof_decl.get(i as usize));
                let new_oneof = candidate
                    .oneof_index
                    .and_then(|i| next.oneof_decl.get(i as usize));
                if field.name != candidate.name
                    || field.r#type != candidate.r#type
                    || field.type_name != candidate.type_name
                    || field.label != candidate.label
                    || field.default_value != candidate.default_value
                    || field.proto3_optional != candidate.proto3_optional
                    || field.json_name != candidate.json_name
                    || old_oneof.map(|o| &o.name) != new_oneof.map(|o| &o.name)
                {
                    return Err(format!(
                        "changed field {}.{}",
                        previous.name(),
                        field.name()
                    ));
                }
            } else if !next.reserved_name.iter().any(|name| name == field.name())
                || !next
                    .reserved_range
                    .iter()
                    .any(|r| r.start() <= field.number() && field.number() < r.end())
            {
                return Err(format!(
                    "removed field without reserving name and number: {}.{}",
                    previous.name(),
                    field.name()
                ));
            }
        }
        for name in &previous.reserved_name {
            if !next.reserved_name.contains(name) {
                return Err(format!("removed reserved name {}.{name}", previous.name()));
            }
        }
        for range in &previous.reserved_range {
            if !next
                .reserved_range
                .iter()
                .any(|r| r.start() <= range.start() && r.end() >= range.end())
            {
                return Err(format!("removed reserved range in {}", previous.name()));
            }
        }
        messages(&previous.nested_type, &next.nested_type)?;
        enums(&previous.enum_type, &next.enum_type)?;
    }
    Ok(())
}

fn compatible(
    old: &prost_types::FileDescriptorProto,
    new: &prost_types::FileDescriptorProto,
) -> Result<(), String> {
    if old.package != new.package || old.syntax != new.syntax {
        return Err("changed package or syntax".into());
    }
    messages(&old.message_type, &new.message_type)?;
    enums(&old.enum_type, &new.enum_type)
}

#[test]
fn current_schema_preserves_the_frozen_v1_contract() {
    compatible(&baseline(), &current()).unwrap();
}

#[test]
fn gate_rejects_field_number_type_label_presence_and_enum_changes() {
    let old = baseline();
    for mutation in 0..12 {
        let mut new = old.clone();
        match mutation {
            0 => new.message_type[0].field[0].number = Some(100),
            1 => new.message_type[0].field[0].r#type = Some(12),
            2 => new.message_type[0].field[0].label = Some(3),
            3 => new.message_type[0].field[0].name = Some("renamed".into()),
            4 => new.message_type[0].field[0].proto3_optional = Some(true),
            5 => new.enum_type[0].value[1].number = Some(100),
            6 => new.message_type[0].reserved_range.clear(),
            7 => new.message_type[0].reserved_name.clear(),
            8 => new.message_type[0].field[5].type_name = Some(".esop.v1.QualitySummary".into()),
            9 => {
                new.message_type[0]
                    .oneof_decl
                    .push(prost_types::OneofDescriptorProto {
                        name: Some("selection".into()),
                        ..Default::default()
                    });
                new.message_type[0].field[0].oneof_index = Some(0);
            }
            10 => new.package = Some("esop.v2".into()),
            11 => new.message_type[1].field[0].r#type = Some(5),
            _ => unreachable!(),
        }
        assert!(
            compatible(&old, &new).is_err(),
            "mutation {mutation} escaped"
        );
    }
}

#[test]
fn deletion_requires_both_reservations_and_additions_are_allowed() {
    let old = baseline();
    let mut new = old.clone();
    let removed = new.message_type[0].field.remove(0);
    assert!(compatible(&old, &new).is_err());
    new.message_type[0]
        .reserved_name
        .push(removed.name().into());
    assert!(compatible(&old, &new).is_err());
    new.message_type[0]
        .reserved_range
        .push(prost_types::descriptor_proto::ReservedRange {
            start: removed.number,
            end: Some(removed.number() + 1),
        });
    compatible(&old, &new).unwrap();

    let mut new = old.clone();
    let mut added = old.message_type[0].field[0].clone();
    added.name = Some("future_label".into());
    added.json_name = Some("futureLabel".into());
    added.number = Some(100);
    new.message_type[0].field.push(added);
    compatible(&old, &new).unwrap();
}
