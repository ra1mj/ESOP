use esop_cfggen::generate;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);
const ARTIFACTS: [&str; 5] = [
    "device_inventory.json",
    "esop_product_config.h",
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

    let build_input: Value =
        serde_json::from_slice(&fs::read(first.join("robot_build_input.json")).unwrap()).unwrap();
    assert_eq!(build_input["process_data"]["pdo_bytes_per_cycle"], 36);
    assert_eq!(build_input["process_data"]["frame_count"], 2);
    assert_eq!(build_input["process_data"]["expected_wkc"], 6);
    assert_eq!(build_input["process_data"]["copy_bytes_per_cycle"], 20);
    assert_eq!(build_input["process_data"]["wire_bytes_per_cycle"], 180);

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
