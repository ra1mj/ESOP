fn main() {
    println!("cargo:rerun-if-changed=../../proto/esop/v1/esop.proto");
    println!("cargo:rerun-if-changed=tests/fixtures/v1_784bf73.proto");

    let protoc = protoc_bin_vendored::protoc_bin_path().expect("vendored protoc is available");
    let out =
        std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo output directory"));
    let mut config = prost_build::Config::new();
    config.protoc_executable(&protoc);
    config.file_descriptor_set_path(out.join("current.bin"));
    config
        .compile_protos(&["../../proto/esop/v1/esop.proto"], &["../../proto"])
        .expect("ESOP Protobuf schema compiles");

    let baseline = out.join("baseline");
    std::fs::create_dir_all(&baseline).expect("baseline output directory");
    let mut config = prost_build::Config::new();
    config.protoc_executable(protoc);
    config.out_dir(&baseline);
    config.file_descriptor_set_path(out.join("baseline.bin"));
    config
        .compile_protos(&["tests/fixtures/v1_784bf73.proto"], &["tests/fixtures"])
        .expect("frozen ESOP Protobuf baseline compiles");
}
