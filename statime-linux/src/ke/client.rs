use std::{error::Error, io, sync::Arc};

use rustls::{pki_types::ServerName, ClientConfig};
use tokio::{io::AsyncReadExt, net::TcpStream};
use tokio_rustls::TlsConnector;

use super::record::{AssociationMode, NextProtocols, PtpKeyRequestMessage, PtpKeyResponseMessage};
use crate::ke::record::Record;

pub async fn fetch_data(
    server_address: &str,
    config: Arc<ClientConfig>,
) -> Result<PtpKeyResponseMessage<'static>, Box<dyn Error>> {
    let request = PtpKeyRequestMessage {
        next_protocol: NextProtocols::ptpv2_1(),
        association_mode: AssociationMode::Group {
            ptp_domain_number: 0,
            sdo_id: 0.try_into().unwrap(),
            subgroup: 0,
        },
    };

    let connector = TlsConnector::from(config);
    let dnsname = ServerName::try_from(server_address.to_owned())?;

    let stream = TcpStream::connect(server_address).await?;
    let mut stream = connector.connect(dnsname, stream).await?;

    request.write(&mut stream).await?;

    // we expect the to receive messages to be smaller than data_buf
    let mut data_buf = vec![0; 4096];
    let mut bytes_received = 0;

    let records = loop {
        bytes_received += stream.read(&mut data_buf[bytes_received..]).await?;
        let mut data = &data_buf[0..bytes_received];
        let records = Record::read_until_eom(&mut data)?;
        if let Some(records) = records {
            break records;
        } else if bytes_received == data_buf.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "NTS message too large to handle",
            )
            .into());
        }
    };

    let records: Vec<_> = records.into_iter().map(|r| r.into_owned()).collect();

    Ok(records.try_into()?)
}
