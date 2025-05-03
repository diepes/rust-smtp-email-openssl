use std::io;
use std::net::ToSocketAddrs;
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
// use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio_rustls::{client::TlsStream, TlsConnector};

pub enum Stream {
    TcpStream(TcpStream),
    TlsStream(TlsStream<TcpStream>),
    None, // Placeholder for no stream as we swap streams
}

pub struct SmtpConnection {
    pub smtp_stream: Stream, // Tcp or Tls stream
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
}
impl SmtpConnection {
    pub fn new(host: &str, port: u16, username: Option<&str>, password: Option<&str>) -> Self {
        SmtpConnection {
            smtp_stream: Stream::None,
            host: host.to_string(),
            port,
            username: username.map(|s| s.to_string()),
            password: password.map(|s| s.to_string()),
        }
    }
    pub async fn connect_to_server(&mut self) -> Result<(), io::Error> {
        // log4::init_log();
        // Resolve the host and connect to the SMTP server
        let addr = (self.host.clone(), self.port)
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Could not resolve host"))?;
        let tcp_stream = TcpStream::connect(addr).await?;
        //let smtp_stream = stream::Stream::new(stream, host, port);
        log::debug!("Connected to SMTP server at {}", addr);
        self.smtp_stream = Stream::TcpStream(tcp_stream);
        Ok(())
    }

    /// Upgrade the existing TCP stream to a TLS stream
    pub async fn switch_to_tls(&mut self) -> io::Result<()> {
        // Configure rustls, root certificates used by Mozilla
        let root_store = rustls::RootCertStore {
            roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
        };

        let config = rustls::ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();
        let connector = TlsConnector::from(Arc::new(config));

        // Perform TLS handshake
        let domain = rustls::pki_types::ServerName::try_from(self.host.clone())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Invalid hostname"))?
            .to_owned();
        // extract current TCP stream, get value by swapping with None
        // This is a workaround to avoid borrowing issues with the TcpStream
        let tls_stream = match std::mem::replace(&mut self.smtp_stream, Stream::None) {
            Stream::TcpStream(tcp) => connector.connect(domain, tcp).await?,
            Stream::TlsStream(tls) => tls,
            Stream::None => return Err(io::Error::new(io::ErrorKind::Other, "Stream is None")),
        };
        self.smtp_stream = Stream::TlsStream(tls_stream);
        log::info!("TLS handshake completed");
        Ok(())
    }

    pub async fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        match &mut self.smtp_stream {
            Stream::TcpStream(s) => s.write(data).await?,
            Stream::TlsStream(s) => s.write(data).await?,
            Stream::None => return Err(io::Error::new(io::ErrorKind::Other, "Stream is None")),
        };
        Ok(data.len())
    }
    pub async fn read(&mut self) -> io::Result<String> {
        // Buffer for reading server responses
        let mut buf: [u8; 1024] = [0; 1024];
        let bytes_read;
        // let s = std::mem::replace(&mut self.smtp_stream, Stream::None);
        // match s {
        match &mut self.smtp_stream {
            Stream::TcpStream(s) => bytes_read = s.read(&mut buf).await?,
            Stream::TlsStream(s) => bytes_read = s.read(&mut buf).await?,
            Stream::None => return Err(io::Error::new(io::ErrorKind::Other, "Stream is None")),
        };
        Ok(String::from_utf8_lossy(&buf[..bytes_read]).to_string())
    }
}
