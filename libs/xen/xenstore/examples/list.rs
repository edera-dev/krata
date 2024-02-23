use futures::executor::block_on;
use xenstore::client::{XsdClient, XsdInterface};
use xenstore::error::Result;
use xenstore::sys::XSD_ERROR_EINVAL;

fn list_recursive(client: &mut XsdClient, level: usize, path: &str) -> Result<()> {
    let children = match block_on(client.list(path)) {
        Ok(children) => children,
        Err(error) => {
            return if error.to_string() == XSD_ERROR_EINVAL.error {
                Ok(())
            } else {
                Err(error)
            }
        }
    };

    for child in children {
        let full = format!("{}/{}", if path == "/" { "" } else { path }, child);
        let value = block_on(client.read_string(full.as_str()))?.expect("expected value");
        println!("{}{} = {:?}", " ".repeat(level), child, value,);
        list_recursive(client, level + 1, full.as_str())?;
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut client = XsdClient::open().await?;
    list_recursive(&mut client, 0, "/")?;
    Ok(())
}
