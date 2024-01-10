use std::alloc::Layout;
use xenclient::boot::ElfLoader;
use xenclient::XenClientError;

fn main() -> Result<(), XenClientError> {
    let boot = ElfLoader::load_file_kernel("/boot/vmlinuz-6.1.0-17-amd64")?;
    let ptr = unsafe { std::alloc::alloc(Layout::from_size_align(128 * 1024 * 1024, 16).unwrap()) };
    boot.load(ptr)?;
    Ok(())
}
