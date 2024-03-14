use std::io::Result;

fn main() -> Result<()> {
    tonic_build::configure().compile(&["proto/krata/control.proto"], &["proto/"])?;
    Ok(())
}
