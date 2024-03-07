use crate::bus::XsdSocket;
use crate::error::{Error, Result};
use crate::sys::{
    XSD_DIRECTORY, XSD_GET_DOMAIN_PATH, XSD_INTRODUCE, XSD_MKDIR, XSD_READ, XSD_RM, XSD_SET_PERMS,
    XSD_TRANSACTION_END, XSD_TRANSACTION_START, XSD_WATCH, XSD_WRITE,
};
use log::trace;
use std::ffi::CString;
use tokio::sync::mpsc::Receiver;
use tokio::sync::mpsc::Sender;

pub const XS_PERM_NONE: u32 = 0x00;
pub const XS_PERM_READ: u32 = 0x01;
pub const XS_PERM_WRITE: u32 = 0x02;
pub const XS_PERM_READ_WRITE: u32 = XS_PERM_READ | XS_PERM_WRITE;

#[derive(Debug, Copy, Clone)]
pub struct XsPermission {
    pub id: u32,
    pub perms: u32,
}

#[derive(Clone)]
pub struct XsdClient {
    pub socket: XsdSocket,
}

impl XsPermission {
    pub fn encode(&self) -> Result<String> {
        let c = match self.perms {
            XS_PERM_READ_WRITE => 'b',
            XS_PERM_WRITE => 'w',
            XS_PERM_READ => 'r',
            XS_PERM_NONE => 'n',
            _ => return Err(Error::InvalidPermissions),
        };
        Ok(format!("{}{}", c, self.id))
    }
}

pub struct XsdWatchHandle {
    id: u32,
    unwatch_sender: Sender<u32>,
    pub receiver: Receiver<String>,
}

impl Drop for XsdWatchHandle {
    fn drop(&mut self) {
        let _ = self.unwatch_sender.try_send(self.id);
    }
}

#[allow(async_fn_in_trait)]
pub trait XsdInterface {
    async fn list(&self, path: &str) -> Result<Vec<String>>;
    async fn read(&self, path: &str) -> Result<Option<Vec<u8>>>;
    async fn read_string(&self, path: &str) -> Result<Option<String>>;
    async fn write(&self, path: &str, data: Vec<u8>) -> Result<bool>;
    async fn write_string(&self, path: &str, data: &str) -> Result<bool>;
    async fn mkdir(&self, path: &str) -> Result<bool>;
    async fn rm(&self, path: &str) -> Result<bool>;
    async fn set_perms(&self, path: &str, perms: &[XsPermission]) -> Result<bool>;

    async fn mknod(&self, path: &str, perms: &[XsPermission]) -> Result<bool> {
        let result1 = self.write_string(path, "").await?;
        let result2 = self.set_perms(path, perms).await?;
        Ok(result1 && result2)
    }
}

impl XsdClient {
    pub async fn open() -> Result<XsdClient> {
        let socket = XsdSocket::open().await?;
        Ok(XsdClient { socket })
    }

    async fn list(&self, tx: u32, path: &str) -> Result<Vec<String>> {
        trace!("list tx={tx} path={path}");
        let response = match self.socket.send(tx, XSD_DIRECTORY, &[path]).await {
            Ok(response) => response,
            Err(error) => {
                if error.is_noent_response() {
                    return Ok(vec![]);
                }
                return Err(error);
            }
        };
        response.parse_string_vec()
    }

    async fn read(&self, tx: u32, path: &str) -> Result<Option<Vec<u8>>> {
        trace!("read tx={tx} path={path}");
        match self.socket.send(tx, XSD_READ, &[path]).await {
            Ok(response) => Ok(Some(response.payload)),
            Err(error) => {
                if error.is_noent_response() {
                    Ok(None)
                } else {
                    Err(error)
                }
            }
        }
    }

    async fn write(&self, tx: u32, path: &str, data: Vec<u8>) -> Result<bool> {
        trace!("write tx={tx} path={path} data={:?}", data);
        let mut buffer = Vec::new();
        let path = CString::new(path)?;
        buffer.extend_from_slice(path.as_bytes_with_nul());
        buffer.extend_from_slice(data.as_slice());
        let response = self
            .socket
            .send_buf(tx, XSD_WRITE, buffer.as_slice())
            .await?;
        response.parse_bool()
    }

    async fn mkdir(&self, tx: u32, path: &str) -> Result<bool> {
        trace!("mkdir tx={tx} path={path}");
        self.socket.send(tx, XSD_MKDIR, &[path]).await?.parse_bool()
    }

    async fn rm(&self, tx: u32, path: &str) -> Result<bool> {
        trace!("rm tx={tx} path={path}");
        let result = self.socket.send(tx, XSD_RM, &[path]).await;
        if let Err(error) = result {
            if error.is_noent_response() {
                return Ok(true);
            }
            return Err(error);
        }
        result.unwrap().parse_bool()
    }

    async fn set_perms(&self, tx: u32, path: &str, perms: &[XsPermission]) -> Result<bool> {
        trace!("set_perms tx={tx} path={path} perms={:?}", perms);
        let mut items: Vec<String> = Vec::new();
        items.push(path.to_string());
        for perm in perms {
            items.push(perm.encode()?);
        }
        let items_str: Vec<&str> = items.iter().map(|x| x.as_str()).collect();
        let response = self.socket.send(tx, XSD_SET_PERMS, &items_str).await?;
        response.parse_bool()
    }

    pub async fn transaction(&self) -> Result<XsdTransaction> {
        trace!("transaction start");
        let response = self.socket.send(0, XSD_TRANSACTION_START, &[""]).await?;
        let str = response.parse_string()?;
        let tx = str.parse::<u32>()?;
        Ok(XsdTransaction {
            client: self.clone(),
            tx,
        })
    }

    pub async fn get_domain_path(&mut self, domid: u32) -> Result<String> {
        let response = self
            .socket
            .send(0, XSD_GET_DOMAIN_PATH, &[&domid.to_string()])
            .await?;
        response.parse_string()
    }

    pub async fn introduce_domain(&mut self, domid: u32, mfn: u64, evtchn: u32) -> Result<bool> {
        trace!("introduce domain domid={domid} mfn={mfn} evtchn={evtchn}");
        let response = self
            .socket
            .send(
                0,
                XSD_INTRODUCE,
                &[
                    domid.to_string().as_str(),
                    mfn.to_string().as_str(),
                    evtchn.to_string().as_str(),
                ],
            )
            .await?;
        response.parse_bool()
    }

    pub async fn watch(&self, path: &str) -> Result<XsdWatchHandle> {
        let (id, receiver, unwatch_sender) = self.socket.add_watch().await?;
        let id_string = id.to_string();
        let _ = self.socket.send(0, XSD_WATCH, &[path, &id_string]).await?;
        Ok(XsdWatchHandle {
            id,
            receiver,
            unwatch_sender,
        })
    }
}

#[derive(Clone)]
pub struct XsdTransaction {
    client: XsdClient,
    tx: u32,
}

impl XsdInterface for XsdClient {
    async fn list(&self, path: &str) -> Result<Vec<String>> {
        self.list(0, path).await
    }

    async fn read(&self, path: &str) -> Result<Option<Vec<u8>>> {
        self.read(0, path).await
    }

    async fn read_string(&self, path: &str) -> Result<Option<String>> {
        match self.read(0, path).await {
            Ok(value) => match value {
                Some(value) => Ok(Some(String::from_utf8(value)?)),
                None => Ok(None),
            },
            Err(error) => Err(error),
        }
    }

    async fn write(&self, path: &str, data: Vec<u8>) -> Result<bool> {
        self.write(0, path, data).await
    }

    async fn write_string(&self, path: &str, data: &str) -> Result<bool> {
        self.write(0, path, data.as_bytes().to_vec()).await
    }

    async fn mkdir(&self, path: &str) -> Result<bool> {
        self.mkdir(0, path).await
    }

    async fn rm(&self, path: &str) -> Result<bool> {
        self.rm(0, path).await
    }

    async fn set_perms(&self, path: &str, perms: &[XsPermission]) -> Result<bool> {
        self.set_perms(0, path, perms).await
    }
}

impl XsdInterface for XsdTransaction {
    async fn list(&self, path: &str) -> Result<Vec<String>> {
        self.client.list(self.tx, path).await
    }

    async fn read(&self, path: &str) -> Result<Option<Vec<u8>>> {
        self.client.read(self.tx, path).await
    }

    async fn read_string(&self, path: &str) -> Result<Option<String>> {
        match self.client.read(self.tx, path).await {
            Ok(value) => match value {
                Some(value) => Ok(Some(String::from_utf8(value)?)),
                None => Ok(None),
            },
            Err(error) => Err(error),
        }
    }

    async fn write(&self, path: &str, data: Vec<u8>) -> Result<bool> {
        self.client.write(self.tx, path, data).await
    }

    async fn write_string(&self, path: &str, data: &str) -> Result<bool> {
        self.client
            .write(self.tx, path, data.as_bytes().to_vec())
            .await
    }

    async fn mkdir(&self, path: &str) -> Result<bool> {
        self.client.mkdir(self.tx, path).await
    }

    async fn rm(&self, path: &str) -> Result<bool> {
        self.client.rm(self.tx, path).await
    }

    async fn set_perms(&self, path: &str, perms: &[XsPermission]) -> Result<bool> {
        self.client.set_perms(self.tx, path, perms).await
    }
}

impl XsdTransaction {
    pub async fn end(&self, abort: bool) -> Result<bool> {
        let abort_str = if abort { "F" } else { "T" };

        trace!("transaction end abort={}", abort);
        self.client
            .socket
            .send(self.tx, XSD_TRANSACTION_END, &[abort_str])
            .await?
            .parse_bool()
    }

    pub async fn commit(&self) -> Result<bool> {
        self.end(false).await
    }

    pub async fn abort(&self) -> Result<bool> {
        self.end(true).await
    }
}
