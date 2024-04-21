use std::io::Result;

fn main() -> Result<()> {
    let mut config = prost_build::Config::new();
    prost_reflect_build::Builder::new()
        .descriptor_pool("crate::DESCRIPTOR_POOL")
        .configure(
            &mut config,
            &[
                "proto/krata/v1/control.proto",
                "proto/krata/idm/transport.proto",
                "proto/krata/idm/internal.proto",
            ],
            &["proto/"],
        )?;
    tonic_build::configure().compile_with_config(
        config,
        &[
            "proto/krata/v1/control.proto",
            "proto/krata/idm/transport.proto",
            "proto/krata/idm/internal.proto",
        ],
        &["proto/"],
    )?;
    Ok(())
}
