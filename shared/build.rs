fn main() {
    tonic_build::configure()
        .compile(&["proto/krata/control.proto"], &["proto"])
        .unwrap();
}
