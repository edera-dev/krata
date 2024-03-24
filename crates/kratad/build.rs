use std::io::Result;

fn main() -> Result<()> {
    prost_build::Config::new()
        .extern_path(".krata.v1.common", "::krata::v1::common")
        .compile_protos(&["proto/kratad/db.proto"], &["proto/", "../../proto"])?;
    Ok(())
}
