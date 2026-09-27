use esop_cfggen::generate;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);
const ARTIFACTS: [&str; 6] = [
    "device_inventory.json",
    "esop_product_config.h",
    "esop_product_config.rs",
    "procbuf_layout.json",
    "product_config.json",
    "robot_build_input.json",
];

struct Fixture {
    root: PathBuf,
    product: PathBuf,
    esi: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("esop-cfggen-{}-{sequence}", std::process::id()));
        if root.exists() {
            fs::remove_dir_all(&root).unwrap();
        }
        fs::create_dir_all(root.join("esi")).unwrap();
        let source =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/examples/sim-dual-axis");
        let product = root.join("product.json");
        let esi = root.join("esi/esop-sim.xml");
        fs::copy(source.join("product.json"), &product).unwrap();
        fs::copy(source.join("esi/esop-sim.xml"), &esi).unwrap();
        Self { root, product, esi }
    }

    fn output(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    fn edit_product(&self, edit: impl FnOnce(&mut Value)) {
        let mut value: Value = serde_json::from_slice(&fs::read(&self.product).unwrap()).unwrap();
        edit(&mut value);
        fs::write(&self.product, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    }

    fn edit_esi(&self, edit: impl FnOnce(String) -> String) {
        let source = fs::read_to_string(&self.esi).unwrap();
        fs::write(&self.esi, edit(source)).unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn artifact_bytes(output: &Path) -> Vec<Vec<u8>> {
    ARTIFACTS
        .iter()
        .map(|name| fs::read(output.join(name)).unwrap())
        .collect()
}

#[test]
fn example_generation_is_deterministic_across_json_and_xml_formatting() {
    let fixture = Fixture::new();
    let first = fixture.output("first");
    let second = fixture.output("second");
    let first_summary = generate(&fixture.product, &first).unwrap();
    assert_eq!(first_summary.artifact_count, ARTIFACTS.len());

    let build_input: Value =
        serde_json::from_slice(&fs::read(first.join("robot_build_input.json")).unwrap()).unwrap();
    assert_eq!(build_input["process_data"]["pdo_bytes_per_cycle"], 36);
    assert_eq!(build_input["process_data"]["frame_count"], 2);
    assert_eq!(build_input["process_data"]["expected_wkc"], 6);
    assert_eq!(build_input["process_data"]["copy_bytes_per_cycle"], 20);
    assert_eq!(build_input["process_data"]["wire_bytes_per_cycle"], 180);

    let inventory: Value =
        serde_json::from_slice(&fs::read(first.join("device_inventory.json")).unwrap()).unwrap();
    assert_eq!(inventory["devices"][0]["mailbox"]["send_address"], 0x1000);
    assert_eq!(inventory["devices"][0]["mailbox"]["send_capacity"], 64);
    assert_eq!(
        inventory["devices"][0]["mailbox"]["send_control_byte"],
        0x26
    );
    assert_eq!(
        inventory["devices"][2]["mailbox"]["receive_address"],
        0x1300
    );

    let product: Value =
        serde_json::from_slice(&fs::read(first.join("product_config.json")).unwrap()).unwrap();
    assert_eq!(product["slaves"][0]["mailbox"]["receive_capacity"], 64);
    assert_eq!(product["slaves"][2]["mailbox"]["send_capacity"], 32);
    assert_eq!(product["slaves"][0]["sii_sync_manager_count"], 4);
    assert_eq!(product["slaves"][0]["sii_enabled_sync_managers"], 15);
    assert_eq!(product["slaves"][0]["sii_fmmu_count"], 2);
    assert_eq!(product["slaves"][0]["sii_fmmu_usages"][0], "outputs");
    assert_eq!(product["slaves"][0]["sii_fmmu_usages"][1], "inputs");
    assert_eq!(product["slaves"][0]["sii_fmmu_usages"][2], "unused");
    assert_eq!(
        product["slaves"][0]["sii_fmmu_usages"]
            .as_array()
            .unwrap()
            .len(),
        16
    );
    assert_eq!(product["slaves"][0]["dc"]["required"], true);
    assert_eq!(product["slaves"][0]["dc"]["reference_clock"], true);
    assert_eq!(product["slaves"][0]["dc"]["op_mode"], "DcSync");
    assert_eq!(product["slaves"][0]["sii_dc_mode"]["name"], "DcSync");
    assert_eq!(
        product["slaves"][0]["sii_dc_mode"]["assign_activate"],
        0x0300
    );
    assert_eq!(
        product["slaves"][0]["dc_sync_timing"]["cycle_time0_ns"],
        1_000_000
    );
    assert_eq!(product["slaves"][0]["dc_sync_timing"]["cycle_time1_ns"], 0);
    assert_eq!(
        product["slaves"][0]["dc_sync_timing"]["assign_activate"],
        0x0300
    );
    assert_eq!(product["slaves"][1]["dc"]["required"], true);
    assert_eq!(product["slaves"][1]["dc"]["reference_clock"], false);
    assert_eq!(product["slaves"][2]["dc"]["required"], false);

    let header = fs::read_to_string(first.join("esop_product_config.h")).unwrap();
    assert!(header.contains("uint16_t mailbox_send_address"));
    assert!(header.contains("uint8_t sii_sync_manager_count"));
    assert!(header.contains("uint16_t sii_enabled_sync_managers"));
    assert!(header.contains("#define ESOP_SII_FMMU_CAPACITY 16u"));
    assert!(header.contains("uint8_t sii_fmmu_count"));
    assert!(header.contains("uint8_t sii_fmmu_usages[ESOP_SII_FMMU_CAPACITY]"));
    assert!(header.contains("uint8_t dc_required"));
    assert!(header.contains("uint8_t dc_reference_clock"));
    assert!(header.contains("const char *dc_op_mode"));
    assert!(header.contains("uint16_t dc_assign_activate"));
    assert!(header.contains("uint8_t has_dc_sync_timing"));
    assert!(header.contains("uint32_t dc_sync_cycle_time1_ns"));
    assert!(header.contains("UINT16_C(0x1000), UINT16_C(64), UINT16_C(0x1100), UINT16_C(64)"));
    assert!(header.contains("4u, UINT16_C(0x000f)"));
    assert!(header.contains("2u, {1u, 2u, 0u, 0u"));

    let rust = fs::read_to_string(first.join("esop_product_config.rs")).unwrap();
    assert!(rust.contains("MailboxConfig::new(0x1000, 64, 0x1100, 64)"));
    assert!(rust.contains("MailboxConfig::new(0x1200, 32, 0x1300, 32)"));
    assert!(rust.contains("sii_sync_manager_count: 4"));
    assert!(rust.contains("sii_enabled_sync_managers: 0x000f"));
    assert!(rust.contains("sii_fmmu_count: 2"));
    assert!(rust.contains("SiiFmmuUsage::Outputs"));
    assert!(rust.contains("SiiFmmuUsage::Inputs"));
    assert!(rust.contains("dc_required: true, dc_reference_clock: true"));
    assert!(rust.contains("dc_required: false, dc_reference_clock: false"));
    assert!(rust.contains("name: \"DcSync\", mode: SiiDcMode"));
    assert!(rust.contains("assign_activate: 0x0300"));
    assert!(rust.contains("dc_sync_timing: Some(DcSyncTiming"));
    assert!(rust.contains("cycle_time1_ns: 0"));

    fixture.edit_product(|_| {});
    let xml = fs::read_to_string(&fixture.esi).unwrap();
    let compact = xml
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<String>();
    fs::write(&fixture.esi, compact).unwrap();
    let second_summary = generate(&fixture.product, &second).unwrap();

    assert_eq!(first_summary.config_sha256, second_summary.config_sha256);
    assert_eq!(artifact_bytes(&first), artifact_bytes(&second));
}

#[test]
fn dc_policy_defaults_hashes_and_invalid_references_are_strict() {
    let fixture = Fixture::new();
    let baseline = generate(&fixture.product, &fixture.output("dc-baseline")).unwrap();
    fixture.edit_product(|product| {
        product["slaves"][1]["dc"]["required"] = Value::Bool(false);
        product["slaves"][1]["dc"]
            .as_object_mut()
            .unwrap()
            .remove("op_mode");
    });
    let changed = generate(&fixture.product, &fixture.output("dc-changed")).unwrap();
    assert_ne!(baseline.config_sha256, changed.config_sha256);

    let fixture = Fixture::new();
    fixture.edit_product(|product| {
        product["slaves"][2].as_object_mut().unwrap().remove("dc");
    });
    let output = fixture.output("dc-default");
    generate(&fixture.product, &output).unwrap();
    let product: Value =
        serde_json::from_slice(&fs::read(output.join("product_config.json")).unwrap()).unwrap();
    assert_eq!(product["slaves"][2]["dc"]["required"], false);
    assert_eq!(product["slaves"][2]["dc"]["reference_clock"], false);

    let fixture = Fixture::new();
    fixture.edit_product(|product| {
        product["slaves"][0]["dc"]["required"] = Value::Bool(false);
    });
    assert!(
        generate(
            &fixture.product,
            &fixture.output("dc-reference-without-required")
        )
        .unwrap_err()
        .to_string()
        .contains("without requiring DC System Time")
    );

    let fixture = Fixture::new();
    fixture.edit_product(|product| {
        product["slaves"][1]["dc"]["reference_clock"] = Value::Bool(true);
    });
    assert!(
        generate(&fixture.product, &fixture.output("dc-duplicate-reference"))
            .unwrap_err()
            .to_string()
            .contains("both select the DC reference clock")
    );

    let fixture = Fixture::new();
    fixture.edit_product(|product| {
        product["slaves"][0]["dc"]
            .as_object_mut()
            .unwrap()
            .remove("op_mode");
    });
    assert!(
        generate(
            &fixture.product,
            &fixture.output("dc-required-without-mode")
        )
        .unwrap_err()
        .to_string()
        .contains("does not select dc.op_mode")
    );

    let fixture = Fixture::new();
    fixture.edit_product(|product| {
        product["slaves"][2]["dc"]["op_mode"] = Value::String("DcSync".to_owned());
    });
    assert!(
        generate(
            &fixture.product,
            &fixture.output("dc-mode-without-required")
        )
        .unwrap_err()
        .to_string()
        .contains("without requiring DC System Time")
    );

    let fixture = Fixture::new();
    fixture.edit_product(|product| {
        product["slaves"][0]["dc"]["op_mode"] = Value::String("Missing".to_owned());
    });
    assert!(
        generate(&fixture.product, &fixture.output("dc-unknown-mode"))
            .unwrap_err()
            .to_string()
            .contains("unknown DC OpMode")
    );

    let fixture = Fixture::new();
    fixture.edit_product(|product| {
        product["slaves"][0]["dc"]["unexpected"] = Value::Bool(true);
    });
    assert!(
        generate(&fixture.product, &fixture.output("dc-unknown-field"))
            .unwrap_err()
            .to_string()
            .contains("unknown field")
    );

    let fixture = Fixture::new();
    fixture.edit_esi(|xml| xml.replacen("#x0300", "#x0100", 1));
    assert!(
        generate(&fixture.product, &fixture.output("dc-invalid-activation"))
            .unwrap_err()
            .to_string()
            .contains("InvalidActivation(1)")
    );
}

#[test]
fn mailbox_metadata_changes_esi_semantic_and_configuration_hashes() {
    fn hashes(fixture: &Fixture, name: &str) -> (String, String) {
        let output = fixture.output(name);
        let summary = generate(&fixture.product, &output).unwrap();
        let inventory: Value =
            serde_json::from_slice(&fs::read(output.join("device_inventory.json")).unwrap())
                .unwrap();
        (
            summary.config_sha256,
            inventory["devices"][0]["esi_semantic_sha256"]
                .as_str()
                .unwrap()
                .to_owned(),
        )
    }

    let fixture = Fixture::new();
    let baseline = hashes(&fixture, "baseline-address");
    fixture.edit_esi(|xml| xml.replacen("StartAddress=\"#x1000\"", "StartAddress=\"#x1010\"", 1));
    let changed = hashes(&fixture, "changed-address");
    assert_ne!(baseline.0, changed.0);
    assert_ne!(baseline.1, changed.1);

    let fixture = Fixture::new();
    let baseline = hashes(&fixture, "baseline-capacity");
    fixture.edit_esi(|xml| xml.replacen("DefaultSize=\"64\"", "DefaultSize=\"48\"", 1));
    let changed = hashes(&fixture, "changed-capacity");
    assert_ne!(baseline.0, changed.0);
    assert_ne!(baseline.1, changed.1);
}

#[test]
fn dc_descriptor_changes_esi_semantic_and_configuration_hashes() {
    fn hashes(fixture: &Fixture, name: &str) -> (String, String) {
        let output = fixture.output(name);
        let summary = generate(&fixture.product, &output).unwrap();
        let inventory: Value =
            serde_json::from_slice(&fs::read(output.join("device_inventory.json")).unwrap())
                .unwrap();
        (
            summary.config_sha256,
            inventory["devices"][0]["esi_semantic_sha256"]
                .as_str()
                .unwrap()
                .to_owned(),
        )
    }

    let fixture = Fixture::new();
    let baseline = hashes(&fixture, "baseline-dc");
    fixture.edit_esi(|xml| xml.replacen("<ShiftTimeSync0>0", "<ShiftTimeSync0>125", 1));
    let changed = hashes(&fixture, "changed-dc");
    assert_ne!(baseline.0, changed.0);
    assert_ne!(baseline.1, changed.1);
}

#[test]
fn missing_coe_or_incomplete_mailbox_fails_before_publication() {
    let fixture = Fixture::new();
    fixture.edit_esi(|xml| xml.replacen("<Mailbox><CoE/></Mailbox>", "", 1));
    assert!(
        generate(&fixture.product, &fixture.output("missing-coe"))
            .unwrap_err()
            .to_string()
            .contains("does not declare Mailbox/CoE support")
    );

    let fixture = Fixture::new();
    fixture.edit_esi(|xml| xml.replacen(" StartAddress=\"#x1000\"", "", 1));
    assert!(
        generate(&fixture.product, &fixture.output("incomplete-mailbox"))
            .unwrap_err()
            .to_string()
            .contains("MBoxOut SyncManager is missing StartAddress")
    );
}

#[test]
fn generation_failure_preserves_previous_output_directory() {
    let fixture = Fixture::new();
    let output = fixture.output("published");
    generate(&fixture.product, &output).unwrap();
    let before = artifact_bytes(&output);

    fixture.edit_product(|product| {
        product["unexpected"] = Value::Bool(true);
    });
    assert!(generate(&fixture.product, &output).is_err());
    assert_eq!(before, artifact_bytes(&output));
}

#[test]
fn successful_replacement_removes_stale_artifacts() {
    let fixture = Fixture::new();
    let output = fixture.output("published");
    generate(&fixture.product, &output).unwrap();
    fs::write(output.join("obsolete-v0.json"), b"stale").unwrap();

    generate(&fixture.product, &output).unwrap();

    assert!(!output.join("obsolete-v0.json").exists());
    assert_eq!(ARTIFACTS.len(), fs::read_dir(output).unwrap().count());
}

#[test]
fn identity_pdo_axis_policy_and_capacity_fail_closed() {
    let fixture = Fixture::new();
    fixture.edit_product(|product| {
        product["slaves"][0]["revision"] = Value::String("0x00000002".to_owned());
    });
    assert!(
        generate(&fixture.product, &fixture.output("identity"))
            .unwrap_err()
            .to_string()
            .contains("matched 0 ESI devices")
    );

    let fixture = Fixture::new();
    fixture.edit_product(|product| {
        product["slaves"][0]["rx_pdos"][0] = Value::String("0x1602".to_owned());
    });
    assert!(
        generate(&fixture.product, &fixture.output("pdo"))
            .unwrap_err()
            .to_string()
            .contains("matched 0 entries")
    );

    let fixture = Fixture::new();
    fixture.edit_product(|product| {
        product["axes"][1]["index"] = Value::from(0);
    });
    assert!(
        generate(&fixture.product, &fixture.output("axis"))
            .unwrap_err()
            .to_string()
            .contains("contiguous")
    );

    let fixture = Fixture::new();
    fixture.edit_product(|product| {
        product["axes"][0]["policy"]["max_velocity_radians_per_second"] = Value::from(0.0);
    });
    assert!(
        generate(&fixture.product, &fixture.output("policy"))
            .unwrap_err()
            .to_string()
            .contains("NonPositiveProductLimit")
    );

    let fixture = Fixture::new();
    fixture.edit_product(|product| {
        product["domains"][0]["process_image_capacity_bytes"] = Value::from(8);
    });
    assert!(
        generate(&fixture.product, &fixture.output("capacity"))
            .unwrap_err()
            .to_string()
            .contains("exceeds declared capacity")
    );
}

#[test]
fn invalid_esi_timeouts_and_op_only_directions_fail_closed() {
    let fixture = Fixture::new();
    fixture.edit_esi(|xml| xml.replace("<PreopTimeout>3500", "<PreopTimeout>0"));
    assert!(
        generate(&fixture.product, &fixture.output("zero-timeout"))
            .unwrap_err()
            .to_string()
            .contains("PreopTimeout must be greater than zero")
    );

    let fixture = Fixture::new();
    fixture.edit_esi(|xml| {
        xml.replace(
            "<Sm Enable=\"1\" OpOnly=\"true\">Outputs</Sm>",
            "<Sm Enable=\"1\" OpOnly=\"true\">Inputs</Sm>",
        )
    });
    assert!(
        generate(&fixture.product, &fixture.output("invalid-op-only"))
            .unwrap_err()
            .to_string()
            .contains("declares OpOnly for non-output direction")
    );
}

#[test]
fn invalid_or_excess_fmmu_usage_descriptors_fail_closed() {
    let fixture = Fixture::new();
    fixture.edit_esi(|xml| xml.replacen("<Fmmu>Outputs</Fmmu>", "<Fmmu>VendorSpecific</Fmmu>", 1));
    assert!(
        generate(&fixture.product, &fixture.output("invalid-fmmu-usage"))
            .unwrap_err()
            .to_string()
            .contains("unsupported Device Fmmu usage")
    );

    let fixture = Fixture::new();
    fixture.edit_esi(|xml| {
        xml.replacen(
            "<Fmmu>Outputs</Fmmu>",
            &"<Fmmu>Outputs</Fmmu>".repeat(16),
            1,
        )
    });
    assert!(
        generate(&fixture.product, &fixture.output("excess-fmmu-usages"))
            .unwrap_err()
            .to_string()
            .contains("exceeding supported capacity 16")
    );
}

#[test]
fn overlapping_ranges_and_unknown_fields_are_rejected_before_publication() {
    let fixture = Fixture::new();
    fixture.edit_product(|product| {
        product["domains"][1]["process_image_offset"] = Value::from(32);
    });
    assert!(
        generate(&fixture.product, &fixture.output("overlap"))
            .unwrap_err()
            .to_string()
            .contains("overlaps")
    );

    let fixture = Fixture::new();
    fixture.edit_product(|product| {
        product["cycle"]["extra"] = Value::from(1);
    });
    assert!(
        generate(&fixture.product, &fixture.output("unknown"))
            .unwrap_err()
            .to_string()
            .contains("unknown field")
    );
}

#[test]
fn duplicate_identities_and_unconfined_esi_paths_are_rejected() {
    let fixture = Fixture::new();
    fixture.edit_product(|product| {
        product["slaves"][1]["station_address"] = product["slaves"][0]["station_address"].clone();
    });
    assert!(
        generate(&fixture.product, &fixture.output("duplicate-slave"))
            .unwrap_err()
            .to_string()
            .contains("duplicate name, position, or station address")
    );

    let fixture = Fixture::new();
    fixture.edit_product(|product| {
        product["axes"][1]["name"] = product["axes"][0]["name"].clone();
    });
    assert!(
        generate(&fixture.product, &fixture.output("duplicate-axis"))
            .unwrap_err()
            .to_string()
            .contains("names must be unique")
    );

    let fixture = Fixture::new();
    fixture.edit_product(|product| {
        product["slaves"][0]["esi"]["path"] = Value::String("../outside.xml".to_owned());
    });
    assert!(
        generate(&fixture.product, &fixture.output("path-escape"))
            .unwrap_err()
            .to_string()
            .contains("confined relative path")
    );

    let fixture = Fixture::new();
    fixture.edit_product(|product| {
        product["product"]["name"] = Value::String("x".repeat(129));
    });
    assert!(
        generate(&fixture.product, &fixture.output("long-name"))
            .unwrap_err()
            .to_string()
            .contains("fit within 128 UTF-8 bytes")
    );
}

#[cfg(unix)]
#[test]
fn esi_symlink_cannot_escape_the_product_directory() {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new();
    let outside = fixture.root.with_extension("outside.xml");
    fs::copy(&fixture.esi, &outside).unwrap();
    fs::remove_file(&fixture.esi).unwrap();
    symlink(&outside, &fixture.esi).unwrap();

    let error = generate(&fixture.product, &fixture.output("symlink-escape"))
        .unwrap_err()
        .to_string();
    let _ = fs::remove_file(outside);
    assert!(error.contains("resolves outside the product directory"));
}

#[test]
fn generated_c_strings_escape_untrusted_product_text() {
    let fixture = Fixture::new();
    fixture.edit_product(|product| {
        product["product"]["name"] = Value::String("quoted \"name\"\n#error injected".to_owned());
    });
    let output = fixture.output("escaped-header");
    generate(&fixture.product, &output).unwrap();

    let header = fs::read_to_string(output.join("esop_product_config.h")).unwrap();
    assert!(header.contains("quoted \\\"name\\\"\\012#error injected"));
    assert!(!header.lines().any(|line| line.starts_with("#error")));

    let rust = fs::read_to_string(output.join("esop_product_config.rs")).unwrap();
    assert!(rust.contains("product_name: \"quoted \\\"name\\\"\\n#error injected\""));
    assert!(!rust.lines().any(|line| line.starts_with("#error")));
}

#[test]
fn esi_direction_width_duplicate_object_and_numeric_errors_fail_closed() {
    let fixture = Fixture::new();
    let xml = fs::read_to_string(&fixture.esi).unwrap();
    let xml = xml.replacen("<RxPdo Sm=\"2\">", "<TxPdo Sm=\"2\">", 1);
    let xml = xml.replacen("</RxPdo>", "</TxPdo>", 1);
    fs::write(&fixture.esi, xml).unwrap();
    assert!(
        generate(&fixture.product, &fixture.output("wrong-direction"))
            .unwrap_err()
            .to_string()
            .contains("selected RxPDO 0x1600, matched 0 entries")
    );

    let fixture = Fixture::new();
    let xml = fs::read_to_string(&fixture.esi).unwrap().replacen(
        "<Index>#x6040</Index><SubIndex>0</SubIndex><BitLen>16</BitLen>",
        "<Index>#x6040</Index><SubIndex>0</SubIndex><BitLen>32</BitLen>",
        1,
    );
    fs::write(&fixture.esi, xml).unwrap();
    assert!(
        generate(&fixture.product, &fixture.output("wrong-width"))
            .unwrap_err()
            .to_string()
            .contains("InvalidEntry(Controlword)")
    );

    let fixture = Fixture::new();
    let xml = fs::read_to_string(&fixture.esi).unwrap();
    let entry = "          <Entry>\n            <Index>#x6040</Index><SubIndex>0</SubIndex><BitLen>16</BitLen>\n            <Name>Controlword</Name><DataType>UINT</DataType>\n          </Entry>\n";
    let xml = xml.replacen(entry, &format!("{entry}{entry}"), 1);
    fs::write(&fixture.esi, xml).unwrap();
    assert!(
        generate(&fixture.product, &fixture.output("duplicate-object"))
            .unwrap_err()
            .to_string()
            .contains("duplicate Rx object 0x6040:0")
    );

    let fixture = Fixture::new();
    let xml = fs::read_to_string(&fixture.esi)
        .unwrap()
        .replacen("#x0000E500", "#xnot-a-number", 1);
    fs::write(&fixture.esi, xml).unwrap();
    assert!(
        generate(&fixture.product, &fixture.output("bad-number"))
            .unwrap_err()
            .to_string()
            .contains("invalid Vendor Id")
    );
}
