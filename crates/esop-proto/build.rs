fn main() {
    println!("cargo:rerun-if-changed=../../proto/esop/v1/esop.proto");

    let protoc = protoc_bin_vendored::protoc_bin_path().expect("vendored protoc is available");
    let mut config = prost_build::Config::new();
    config.protoc_executable(protoc);
    config
        .compile_protos(&["../../proto/esop/v1/esop.proto"], &["../../proto"])
        .expect("ESOP Protobuf schema compiles");
}
