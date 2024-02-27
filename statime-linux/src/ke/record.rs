use std::{
    borrow::Cow,
    fmt::{self, Display, Formatter},
    io,
    net::{Ipv4Addr, Ipv6Addr},
    ops::Deref,
};

use statime::config::SdoId;
use tokio::io::{AsyncWrite, AsyncWriteExt};

/// Error during parsing of a record
#[derive(Debug, Clone)]
pub enum RecordParseError {
    InvalidRecordLength,
    MissingCriticalBit,
    InvalidSdoId,
    UnknownRecordType(u16),
    UnknownAssociationType(u16),
    UnexpectedExtraBytes,
    UnexpectedRecord(Record<'static>),
    MissingRecord(RecordType),
}

impl Display for RecordParseError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        use RecordParseError::*;

        match self {
            InvalidRecordLength => write!(f, "Invalid record length"),
            MissingCriticalBit => write!(f, "Missing critical bit for record that requires it"),
            InvalidSdoId => write!(f, "Found invalid sdoId"),
            UnknownRecordType(r) => write!(f, "Found unknown record of type '{}'", r),
            UnknownAssociationType(a) => write!(f, "Found unknown assocation type '{}'", a),
            UnexpectedExtraBytes => {
                write!(f, "Found unexpected extra bytes that cannot parse a record")
            }
            UnexpectedRecord(_) => write!(f, "A record of an unexpected type was found"),
            MissingRecord(t) => write!(
                f,
                "An expected record of type {:?} was missing in a container",
                t
            ),
        }
    }
}

impl std::error::Error for RecordParseError {}

/// Parsing helper function for getting the next byte and updating the slice
/// Caller must make sure the input is large enough.
fn next_u8(d: &mut &[u8]) -> u8 {
    let res = d[0];
    *d = &d[1..];
    res
}

/// Parsing helper function for getting the next u16 and updating the slice.
/// Caller must make sure the input is large enough.
fn next_u16(d: &mut &[u8]) -> u16 {
    let res = u16::from_be_bytes([d[0], d[1]]);
    *d = &d[2..];
    res
}

/// Parsing helper function for getting the next u32 and updating the slice.
/// Caller must make sure the input is large enough.
fn next_u32(d: &mut &[u8]) -> u32 {
    let res = u32::from_be_bytes([d[0], d[1], d[2], d[3]]);
    *d = &d[4..];
    res
}

/// Get an array of size `COUNT` filled with u16s and updates the input slice.
/// Caller must make sure the input is large enough.
fn next_u16s<const COUNT: usize>(d: &mut &[u8]) -> [u16; COUNT] {
    let mut results = [0; COUNT];
    next_u16s_into(d, &mut results);
    results
}

/// Reads u16 values into the target, filling it up. Updates the input slice.
/// Caller must make sure that the target is small enough and the data input is
/// large enough.
fn next_u16s_into(d: &mut &[u8], target: &mut [u16]) {
    for t in target {
        *t = next_u16(d);
    }
}

/// Ensure that the record length matches the expected length
fn validate_record_length(record: &[u8], expected_length: usize) -> Result<(), RecordParseError> {
    if record.len() != expected_length {
        return Err(RecordParseError::InvalidRecordLength);
    }

    Ok(())
}

/// Variants of the NTS-KE error records (i.e. records in the NTS-KE message)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorRecord {
    UnrecognizedCriticalRecord,
    BadRequest,
    InternalServerError,
    NotAuthorized,
    GrantorNotRegistered,
    Unassigned(u16),
    Reserved(u16),
}

impl ErrorRecord {
    pub fn from_error_code(code: u16) -> ErrorRecord {
        use ErrorRecord::*;

        match code {
            0 => UnrecognizedCriticalRecord,
            1 => BadRequest,
            2 => InternalServerError,
            3 => NotAuthorized,
            4 => GrantorNotRegistered,
            i @ 5..=32767 => Unassigned(i),
            other => Reserved(other),
        }
    }

    pub fn as_error_code(&self) -> u16 {
        use ErrorRecord::*;

        match self {
            UnrecognizedCriticalRecord => 0,
            BadRequest => 1,
            InternalServerError => 2,
            NotAuthorized => 3,
            GrantorNotRegistered => 4,
            Unassigned(i) => *i,
            Reserved(i) => *i,
        }
    }

    async fn write(&self, mut w: impl AsyncWrite + Unpin) -> io::Result<usize> {
        w.write_u16(RecordType::Error.raw_record_type()).await?;
        w.write_u16(2).await?;
        let code = self.as_error_code();
        w.write_u16(code).await?;
        Ok(6)
    }
}

/// List of next protocols
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NextProtocols(Vec<NextProtocol>);

impl Deref for NextProtocols {
    type Target = Vec<NextProtocol>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl NextProtocols {
    async fn write(&self, mut w: impl AsyncWrite + Unpin) -> io::Result<usize> {
        // write record type
        w.write_u16(RecordType::NextProtocol.raw_record_type())
            .await?;

        // write length of record
        w.write_u16((self.0.len() * 2) as u16).await?;

        let mut bytes_written = 4;

        // write next protocols array
        for n in self.0.iter() {
            w.write_u16(n.as_u16()).await?;
            bytes_written += 2;
        }

        Ok(bytes_written)
    }

    pub fn ptpv2_1() -> NextProtocols {
        NextProtocols(vec![NextProtocol::Ptpv2_1])
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NextProtocol {
    Ntpv4,
    Ptpv2_1,
    Unassigned(u16),
    Experimental(u16),
}

impl NextProtocol {
    fn as_u16(&self) -> u16 {
        match self {
            NextProtocol::Ntpv4 => 0,
            NextProtocol::Ptpv2_1 => 1,
            NextProtocol::Unassigned(u) => *u,
            NextProtocol::Experimental(u) => *u,
        }
    }

    fn from_u16(np: u16) -> NextProtocol {
        match np {
            0 => NextProtocol::Ntpv4,
            1 => NextProtocol::Ptpv2_1,
            2..=32767 => NextProtocol::Unassigned(np),
            _ => NextProtocol::Experimental(np),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupportedMacAlgorithms(Vec<u16>);

impl SupportedMacAlgorithms {
    async fn write(&self, mut w: impl AsyncWrite + Unpin) -> io::Result<usize> {
        // write record type
        w.write_u16(RecordType::SupportedMacAlgorithms.raw_record_type())
            .await?;

        // write record length
        w.write_u16((self.0.len() * 2) as u16).await?;

        // write the supported mac algorithms
        let mut bytes_written = 4;
        for a in self.0.iter() {
            w.write_u16(*a).await?;
            bytes_written += 2;
        }

        Ok(bytes_written)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParameterSet<'a> {
    pub security_assocation: SecurityAssocation<'a>,
    pub validity_period: ValidityPeriod,
}

impl<'a> ParameterSet<'a> {
    pub fn into_owned(self) -> ParameterSet<'static> {
        ParameterSet {
            security_assocation: self.security_assocation.into_owned(),
            validity_period: self.validity_period,
        }
    }

    async fn write(
        &self,
        mut w: impl AsyncWrite + Unpin,
        as_next_params: bool,
    ) -> io::Result<usize> {
        // write record type
        w.write_u16(if as_next_params {
            RecordType::NextParameters.raw_record_type()
        } else {
            RecordType::CurrentParameters.raw_record_type()
        })
        .await?;

        // store records in buf temporarily to determine length
        let mut buf = vec![];
        let mut cur = std::io::Cursor::new(&mut buf);
        self.security_assocation.write(&mut cur).await?;
        self.validity_period.write(&mut cur).await?;

        // write record length
        w.write_u16(buf.len() as u16).await?;

        // write record content
        w.write_all(&buf).await?;

        Ok(buf.len() + 4)
    }
}

impl<'a> TryFrom<Vec<Record<'a>>> for ParameterSet<'a> {
    type Error = RecordParseError;

    fn try_from(value: Vec<Record<'a>>) -> Result<Self, Self::Error> {
        let mut security_assocation = None;
        let mut validity_period = None;
        for item in value {
            match item {
                Record::SecurityAssociation(s) => {
                    if security_assocation.is_some() {
                        return Err(RecordParseError::UnexpectedRecord(
                            Record::SecurityAssociation(s.into_owned()),
                        ));
                    }
                    security_assocation.replace(s);
                }
                Record::ValidityPeriod(v) => {
                    if validity_period.is_some() {
                        return Err(RecordParseError::UnexpectedRecord(Record::ValidityPeriod(
                            v,
                        )));
                    }
                    validity_period.replace(v);
                }
                Record::EndOfMessage => {}
                _ => {
                    return Err(RecordParseError::UnexpectedRecord(item.into_owned()));
                }
            }
        }

        let Some(security_assocation) = security_assocation else {
            return Err(RecordParseError::MissingRecord(
                RecordType::SecurityAssocation,
            ));
        };
        let Some(validity_period) = validity_period else {
            return Err(RecordParseError::MissingRecord(RecordType::ValidityPeriod));
        };
        Ok(ParameterSet {
            security_assocation,
            validity_period,
        })
    }
}

impl<'a> From<ParameterSet<'a>> for Vec<Record<'a>> {
    fn from(value: ParameterSet<'a>) -> Self {
        vec![
            Record::SecurityAssociation(value.security_assocation),
            Record::ValidityPeriod(value.validity_period),
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssociationMode {
    Group {
        ptp_domain_number: u8,
        sdo_id: SdoId,
        subgroup: u16,
    },
    Ipv4(Ipv4Addr),
    Ipv6(Ipv6Addr),
    Mac([u8; 6]),
    PortIdentity([u8; 10]),
}

impl AssociationMode {
    fn from_data(mut record: &[u8]) -> Result<Self, RecordParseError> {
        if record.len() < 2 {
            return Err(RecordParseError::InvalidRecordLength);
        }

        let assoc_type = next_u16(&mut record);
        let mode = match assoc_type {
            0 => {
                validate_record_length(record, 5)?;

                // top 4 bits in record[1] should be 0
                if record[1] & 0b11110000 != 0 {
                    return Err(RecordParseError::InvalidSdoId);
                }

                // TODO: spec says offset of subgroup should be 4, but we assume it starts right
                // after the sdo id at offset 3
                AssociationMode::Group {
                    ptp_domain_number: next_u8(&mut record),
                    sdo_id: next_u16(&mut record).try_into().unwrap(),
                    subgroup: next_u16(&mut record),
                }
            }
            1 => {
                validate_record_length(record, 4)?;
                AssociationMode::Ipv4(Ipv4Addr::new(record[3], record[2], record[1], record[0]))
            }
            2 => {
                validate_record_length(record, 16)?;
                let addr = next_u16s::<8>(&mut record);
                AssociationMode::Ipv6(Ipv6Addr::new(
                    addr[7], addr[6], addr[5], addr[4], addr[3], addr[2], addr[1], addr[0],
                ))
            }
            3 => {
                validate_record_length(record, 6)?;
                let mut b = [0; 6];
                // insert in reverse order
                b.iter_mut()
                    .rev()
                    .for_each(|dest| *dest = next_u8(&mut record));
                AssociationMode::Mac(b)
            }
            4 => {
                validate_record_length(record, 10)?;
                let mut b = [0; 10];
                b.iter_mut()
                    .rev()
                    .for_each(|dest| *dest = next_u8(&mut record));
                AssociationMode::PortIdentity(b)
            }
            _ => {
                return Err(RecordParseError::UnknownAssociationType(assoc_type));
            }
        };

        Ok(mode)
    }

    async fn write(&self, mut w: impl AsyncWrite + Unpin) -> io::Result<usize> {
        // write record type
        w.write_u16(RecordType::AssociationMode.raw_record_type())
            .await?;

        // write record length (variable length based on type + 2 for the mode type u16)
        let record_len = match self {
            AssociationMode::Group { .. } => 5,
            AssociationMode::Ipv4(_) => 4,
            AssociationMode::Ipv6(_) => 16,
            AssociationMode::Mac(_) => 6,
            AssociationMode::PortIdentity(_) => 10,
        } + 2;
        w.write_u16(record_len).await?;

        // write the content of the assocation mode
        match self {
            AssociationMode::Group {
                ptp_domain_number,
                sdo_id,
                subgroup,
            } => {
                w.write_u16(0).await?;
                w.write_u8(*ptp_domain_number).await?;
                w.write_u16((*sdo_id).into()).await?;
                w.write_u16(*subgroup).await?;
            }
            AssociationMode::Ipv4(addr) => {
                w.write_u16(1).await?;
                w.write_u32(u32::from(*addr)).await?;
            }
            AssociationMode::Ipv6(addr) => {
                w.write_u16(2).await?;
                w.write_u128(u128::from(*addr)).await?;
            }
            AssociationMode::Mac(mac) => {
                w.write_u16(3).await?;
                for b in mac.iter().rev() {
                    w.write_u8(*b).await?;
                }
            }
            AssociationMode::PortIdentity(id) => {
                w.write_u16(4).await?;
                for b in id.iter().rev() {
                    w.write_u8(*b).await?;
                }
            }
        }

        Ok(record_len as usize + 4)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityAssocation<'a> {
    spp: u8,
    iat: u16,
    key_id: u32,
    key: Cow<'a, [u8]>,
}

impl<'a> SecurityAssocation<'a> {
    pub fn from_key_data(key_id: u32, data: &'a [u8]) -> SecurityAssocation<'a> {
        SecurityAssocation {
            spp: 0,
            iat: 0,
            key_id,
            key: Cow::Borrowed(data),
        }
    }

    pub fn into_key_data(self) -> Vec<u8> {
        self.key.into_owned()
    }

    pub fn into_owned(self) -> SecurityAssocation<'static> {
        SecurityAssocation {
            spp: self.spp,
            iat: self.iat,
            key_id: self.key_id,
            key: Cow::Owned(self.key.into_owned()),
        }
    }

    fn from_data(mut record: &[u8]) -> Result<SecurityAssocation, RecordParseError> {
        if record.len() < 9 {
            return Err(RecordParseError::InvalidRecordLength);
        }

        let spp = next_u8(&mut record);
        let iat = next_u16(&mut record);
        let key_id = next_u32(&mut record);
        let key_length = next_u16(&mut record) as usize;
        validate_record_length(record, key_length)?;

        // note: we copy the key at this point
        let mut key = vec![0; key_length];
        key.copy_from_slice(record);
        Ok(SecurityAssocation {
            spp,
            iat,
            key_id,
            key: Cow::Owned(key),
        })
    }

    async fn write(&self, mut w: impl AsyncWrite + Unpin) -> std::io::Result<usize> {
        // write record type
        w.write_u16(RecordType::SecurityAssocation.raw_record_type())
            .await?;

        // write record length
        let record_len = 9 + self.key.len();
        w.write_u16(record_len as u16).await?;

        // write record data
        w.write_u8(self.spp).await?;
        w.write_u16(self.iat).await?;
        w.write_u32(self.key_id).await?;
        w.write_u16(self.key.len() as u16).await?;
        w.write_all(&self.key).await?;

        Ok(4 + record_len)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidityPeriod {
    pub lifetime: u32,
    pub update_period: u32,
    pub grace_period: u32,
}

impl ValidityPeriod {
    fn from_data(mut record: &[u8]) -> Result<ValidityPeriod, RecordParseError> {
        validate_record_length(record, 12)?;

        let lifetime = next_u32(&mut record);
        let update_period = next_u32(&mut record);
        let grace_period = next_u32(&mut record);
        Ok(ValidityPeriod {
            lifetime,
            update_period,
            grace_period,
        })
    }

    async fn write(&self, mut w: impl AsyncWrite + Unpin) -> io::Result<usize> {
        // write record type
        w.write_u16(RecordType::ValidityPeriod.raw_record_type())
            .await?;

        // write record length
        w.write_u16(12).await?;

        // write record content
        w.write_u32(self.lifetime).await?;
        w.write_u32(self.update_period).await?;
        w.write_u32(self.grace_period).await?;

        Ok(16)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum RecordType {
    EndOfMessage = 0,
    NextProtocol = 1,
    Error = 2,
    AssociationMode = 1024,
    SupportedMacAlgorithms = 1033,
    CurrentParameters = 1025,
    NextParameters = 1027,
    SecurityAssocation = 1030,
    ValidityPeriod = 1037,
}

impl RecordType {
    pub fn raw_record_type(&self) -> u16 {
        use RecordType::*;
        let critical = matches!(self, EndOfMessage | NextProtocol | Error);
        *self as u16 + if critical { 0x8000 } else { 0 }
    }
}

impl TryFrom<u16> for RecordType {
    type Error = ();

    fn try_from(value: u16) -> Result<Self, <Self as TryFrom<u16>>::Error> {
        use RecordType::*;

        match value {
            x if x == EndOfMessage as u16 => Ok(EndOfMessage),
            x if x == NextProtocol as u16 => Ok(NextProtocol),
            x if x == Error as u16 => Ok(Error),
            x if x == AssociationMode as u16 => Ok(AssociationMode),
            x if x == SupportedMacAlgorithms as u16 => Ok(SupportedMacAlgorithms),
            x if x == CurrentParameters as u16 => Ok(CurrentParameters),
            x if x == NextParameters as u16 => Ok(NextParameters),
            x if x == SecurityAssocation as u16 => Ok(SecurityAssocation),
            x if x == ValidityPeriod as u16 => Ok(ValidityPeriod),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Record<'a> {
    EndOfMessage,
    NextProtocol(NextProtocols),
    Error(ErrorRecord),
    AssociationMode(AssociationMode),
    SupportedMacAlgorithms(SupportedMacAlgorithms),
    CurrentParameters(ParameterSet<'a>),
    NextParameters(ParameterSet<'a>),
    SecurityAssociation(SecurityAssocation<'a>),
    ValidityPeriod(ValidityPeriod),
}

impl<'a> Record<'a> {
    /// Returns if this record is an end of message record.
    pub fn is_eom(&self) -> bool {
        matches!(self, Record::EndOfMessage)
    }

    /// Create a copy with a static lifetime.
    pub fn into_owned(self) -> Record<'static> {
        match self {
            Record::SecurityAssociation(a) => Record::SecurityAssociation(a.into_owned()),
            Record::CurrentParameters(p) => Record::CurrentParameters(p.into_owned()),
            Record::NextParameters(p) => Record::NextParameters(p.into_owned()),
            Record::AssociationMode(am) => Record::AssociationMode(am),
            Record::EndOfMessage => Record::EndOfMessage,
            Record::Error(e) => Record::Error(e),
            Record::NextProtocol(p) => Record::NextProtocol(p),
            Record::SupportedMacAlgorithms(sma) => Record::SupportedMacAlgorithms(sma),
            Record::ValidityPeriod(vp) => Record::ValidityPeriod(vp),
        }
    }

    /// Helper function that reads the record body after the header and full
    /// record data was read If a record is of an unknown type and the
    /// critical bit is not set, no error will be given but no record would
    /// be returned.
    fn from_decoded_header(
        critical: bool,
        record_type: u16,
        mut record: &'a [u8],
    ) -> Result<Option<Record>, RecordParseError> {
        macro_rules! assert_critical_bit {
            () => {
                if !critical {
                    return Err(RecordParseError::MissingCriticalBit);
                }
            };
        }

        match record_type.try_into() {
            Ok(RecordType::EndOfMessage) => {
                // end of message record
                assert_critical_bit!();
                validate_record_length(record, 0)?;
                Ok(Some(Record::EndOfMessage))
            }
            Ok(RecordType::NextProtocol) => {
                assert_critical_bit!();
                if record.len() % 2 != 0 {
                    return Err(RecordParseError::InvalidRecordLength);
                }
                let mut next_protocols = vec![0; record.len() / 2];
                next_u16s_into(&mut record, &mut next_protocols);
                let next_protocols = next_protocols
                    .into_iter()
                    .map(NextProtocol::from_u16)
                    .collect();
                Ok(Some(Record::NextProtocol(NextProtocols(next_protocols))))
            }
            Ok(RecordType::Error) => {
                assert_critical_bit!();
                validate_record_length(record, 2)?;
                let error_code = next_u16(&mut record);
                Ok(Some(Record::Error(ErrorRecord::from_error_code(
                    error_code,
                ))))
            }
            Ok(RecordType::AssociationMode) => {
                let mode = AssociationMode::from_data(record)?;
                Ok(Some(Record::AssociationMode(mode)))
            }
            Ok(RecordType::CurrentParameters) => {
                let res = Self::read_all(&mut record)?;
                Ok(Some(Record::CurrentParameters(res.try_into()?)))
            }
            Ok(RecordType::NextParameters) => {
                let res = Self::read_all(&mut record)?;
                Ok(Some(Record::NextParameters(res.try_into()?)))
            }
            Ok(RecordType::SecurityAssocation) => {
                let assoc = SecurityAssocation::from_data(record)?;
                Ok(Some(Record::SecurityAssociation(assoc)))
            }
            Ok(RecordType::SupportedMacAlgorithms) => {
                if record.len() % 2 != 0 {
                    return Err(RecordParseError::InvalidRecordLength);
                }

                let mut supported = vec![0; record.len() / 2];
                next_u16s_into(&mut record, &mut supported);
                Ok(Some(Record::SupportedMacAlgorithms(
                    SupportedMacAlgorithms(supported),
                )))
            }
            Ok(RecordType::ValidityPeriod) => {
                // validity period record
                let val_period = ValidityPeriod::from_data(record)?;
                Ok(Some(Record::ValidityPeriod(val_period)))
            }
            Err(_) => {
                // unknown record
                if critical {
                    Err(RecordParseError::UnknownRecordType(record_type))
                } else {
                    Ok(None)
                }
            }
        }
    }

    /// Read a single record from a buffer, updating the buffer so it only
    /// refers to unparsed data. If not enough data was read for a complete
    /// record, nothing will be changed. If a record is unknown and no
    /// critical bit is set no error will be given, but no record will be
    /// returned either. If the critical bit is set an error will be
    /// returned for unknown records.
    pub fn from_buffer(buf: &mut &'a [u8]) -> Result<Option<Record<'a>>, RecordParseError> {
        if buf.len() < 4 {
            return Ok(None);
        }

        let mut data = *buf;

        let raw_record_type = next_u16(&mut data);
        let critical = raw_record_type & 0x8000 != 0;
        let record_type = raw_record_type & !0x8000;
        let record_length = next_u16(&mut data) as usize;

        if data.len() >= record_length {
            let data = &data[0..record_length];
            let res = Self::from_decoded_header(critical, record_type, data)?;
            // remove the parsed data from the buffer
            *buf = &buf[(4 + record_length)..];
            Ok(res)
        } else {
            Ok(None)
        }
    }

    /// Read all records until the end-of-message record, but only return them
    /// if the actual end-of-message record was found, otherwise we assume we
    /// don't have enough data to fully parse the records yet
    pub fn read_until_eom(buf: &mut &'a [u8]) -> Result<Option<Vec<Record<'a>>>, RecordParseError> {
        let mut data = *buf;
        let mut records = vec![];

        while data.len() >= 4 {
            if let Some(rec) = Self::from_buffer(&mut data)? {
                if rec.is_eom() {
                    *buf = data;
                    return Ok(Some(records));
                }
                records.push(rec);
            }
        }

        Ok(None)
    }

    /// Read all records in the buffer and return them
    fn read_all(buf: &mut &'a [u8]) -> Result<Vec<Record<'a>>, RecordParseError> {
        let mut records = vec![];

        while buf.len() >= 4 {
            if let Some(rec) = Self::from_buffer(buf)? {
                records.push(rec);
            }
        }

        if !buf.is_empty() {
            Err(RecordParseError::UnexpectedExtraBytes)
        } else {
            Ok(records)
        }
    }

    pub async fn write(&self, mut w: impl AsyncWrite + Unpin) -> std::io::Result<usize> {
        let mut bytes_written = 0;

        bytes_written += match self {
            Record::EndOfMessage => {
                w.write_u16(RecordType::EndOfMessage.raw_record_type())
                    .await?;
                w.write_u16(0u16).await?;
                4
            }
            Record::NextProtocol(np) => np.write(w).await?,
            Record::Error(e) => e.write(w).await?,
            Record::AssociationMode(am) => am.write(w).await?,
            Record::SupportedMacAlgorithms(alg) => alg.write(w).await?,
            Record::CurrentParameters(params) => params.write(w, false).await?,
            Record::NextParameters(params) => params.write(w, true).await?,
            Record::SecurityAssociation(sec) => sec.write(w).await?,
            Record::ValidityPeriod(vp) => vp.write(w).await?,
        };

        Ok(bytes_written)
    }

    pub async fn write_all(
        data: &Vec<Record<'_>>,
        mut w: impl AsyncWrite + Unpin,
        append_eom: bool,
    ) -> std::io::Result<usize> {
        let mut bytes_written = 0;
        for rec in data {
            bytes_written += rec.write(&mut w).await?;
        }

        if append_eom {
            bytes_written += Record::EndOfMessage.write(&mut w).await?;
        }

        Ok(bytes_written)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct PtpKeyRequestMessage {
    pub next_protocol: NextProtocols,
    pub association_mode: AssociationMode,
}

impl PtpKeyRequestMessage {
    pub async fn write(&self, mut w: impl AsyncWrite + Unpin) -> io::Result<usize> {
        let mut bytes_written = 0;
        bytes_written += self.next_protocol.write(&mut w).await?;
        bytes_written += self.association_mode.write(&mut w).await?;
        bytes_written += Record::EndOfMessage.write(&mut w).await?;

        Ok(bytes_written)
    }
}

impl<'a> TryFrom<Vec<Record<'a>>> for PtpKeyRequestMessage {
    type Error = RecordParseError;

    fn try_from(value: Vec<Record<'a>>) -> Result<Self, Self::Error> {
        let mut next_protocol = None;
        let mut association_mode = None;
        for item in value {
            match item {
                Record::NextProtocol(p) => {
                    if next_protocol.is_some() {
                        return Err(RecordParseError::UnexpectedRecord(Record::NextProtocol(p)));
                    }
                    next_protocol.replace(p);
                }
                Record::AssociationMode(am) => {
                    if association_mode.is_some() {
                        return Err(RecordParseError::UnexpectedRecord(Record::AssociationMode(
                            am,
                        )));
                    }
                    association_mode.replace(am);
                }
                Record::EndOfMessage => {}
                _ => {
                    return Err(RecordParseError::UnexpectedRecord(item.into_owned()));
                }
            }
        }

        let Some(next_protocol) = next_protocol else {
            return Err(RecordParseError::MissingRecord(RecordType::NextProtocol));
        };

        let Some(association_mode) = association_mode else {
            return Err(RecordParseError::MissingRecord(RecordType::AssociationMode));
        };

        Ok(PtpKeyRequestMessage {
            next_protocol,
            association_mode,
        })
    }
}

impl From<PtpKeyRequestMessage> for Vec<Record<'static>> {
    fn from(val: PtpKeyRequestMessage) -> Self {
        vec![
            Record::NextProtocol(val.next_protocol),
            Record::AssociationMode(val.association_mode),
        ]
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct PtpKeyResponseMessage<'a> {
    pub next_protocol: NextProtocols,
    pub current_parameters: ParameterSet<'a>,
    pub next_parameters: Option<ParameterSet<'a>>,
}

impl<'a> PtpKeyResponseMessage<'a> {
    pub async fn write(&self, mut w: impl AsyncWrite + Unpin) -> io::Result<usize> {
        let mut bytes_written = 0;
        bytes_written += self.next_protocol.write(&mut w).await?;
        bytes_written += self.current_parameters.write(&mut w, false).await?;

        if let Some(np) = &self.next_parameters {
            bytes_written += np.write(&mut w, true).await?;
        }

        bytes_written += Record::EndOfMessage.write(&mut w).await?;

        Ok(bytes_written)
    }
}

impl<'a> TryFrom<Vec<Record<'a>>> for PtpKeyResponseMessage<'a> {
    type Error = RecordParseError;

    fn try_from(value: Vec<Record<'a>>) -> Result<PtpKeyResponseMessage<'a>, Self::Error> {
        let mut next_protocol = None;
        let mut current_parameters: Option<ParameterSet<'a>> = None;
        let mut next_parameters: Option<ParameterSet<'a>> = None;
        for item in value {
            match item {
                Record::NextProtocol(p) => {
                    if next_protocol.is_some() {
                        return Err(RecordParseError::UnexpectedRecord(Record::NextProtocol(p)));
                    }
                    next_protocol.replace(p);
                }
                Record::CurrentParameters(p) => {
                    if current_parameters.is_some() {
                        return Err(RecordParseError::UnexpectedRecord(
                            Record::CurrentParameters(p.into_owned()),
                        ));
                    }
                    current_parameters.replace(p);
                }
                Record::NextParameters(p) => {
                    if next_parameters.is_some() {
                        return Err(RecordParseError::UnexpectedRecord(Record::NextParameters(
                            p.into_owned(),
                        )));
                    }
                    next_parameters.replace(p);
                }
                Record::EndOfMessage => {}
                _ => {
                    return Err(RecordParseError::UnexpectedRecord(item.into_owned()));
                }
            }
        }

        let Some(next_protocol) = next_protocol else {
            return Err(RecordParseError::MissingRecord(RecordType::NextProtocol));
        };

        let Some(current_parameters) = current_parameters else {
            return Err(RecordParseError::MissingRecord(
                RecordType::CurrentParameters,
            ));
        };

        Ok(PtpKeyResponseMessage {
            next_protocol,
            current_parameters,
            next_parameters,
        })
    }
}

impl<'a> From<PtpKeyResponseMessage<'a>> for Vec<Record<'a>> {
    fn from(val: PtpKeyResponseMessage<'a>) -> Self {
        let mut v = vec![
            Record::NextProtocol(val.next_protocol),
            Record::CurrentParameters(val.current_parameters),
        ];
        if let Some(next_parameters) = val.next_parameters {
            v.push(Record::NextParameters(next_parameters));
        }

        v
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[tokio::test]
    async fn write_read_simple_records() {
        let mut buf = vec![];

        let rec = Record::EndOfMessage;
        let written = rec.write(Cursor::new(&mut buf)).await.unwrap();
        assert_eq!(written, buf.len());
        let result = Record::from_buffer(&mut &buf[..]).unwrap();
        assert_eq!(result, Some(rec));

        buf.clear();
        let rec = Record::AssociationMode(AssociationMode::Group {
            ptp_domain_number: 7,
            sdo_id: SdoId::try_from(11).unwrap(),
            subgroup: 21,
        });
        let written = rec.write(Cursor::new(&mut buf)).await.unwrap();
        assert_eq!(written, buf.len());
        let result = Record::from_buffer(&mut &buf[..]).unwrap();
        assert_eq!(result, Some(rec));

        buf.clear();
        let rec = Record::ValidityPeriod(ValidityPeriod {
            lifetime: 42,
            update_period: 43,
            grace_period: 44,
        });
        let written = rec.write(Cursor::new(&mut buf)).await.unwrap();
        assert_eq!(written, buf.len());
        let result = Record::from_buffer(&mut &buf[..]).unwrap();
        assert_eq!(result, Some(rec));

        buf.clear();
        let rec = Record::SecurityAssociation(SecurityAssocation::from_key_data(
            11,
            &[1, 2, 3, 4, 5, 6, 7, 8, 9, 0],
        ));
        let written = rec.write(Cursor::new(&mut buf)).await.unwrap();
        assert_eq!(written, buf.len());
        let result = Record::from_buffer(&mut &buf[..]).unwrap();
        assert_eq!(result, Some(rec));

        buf.clear();
        let rec = Record::NextProtocol(NextProtocols::ptpv2_1());
        let written = rec.write(Cursor::new(&mut buf)).await.unwrap();
        assert_eq!(written, buf.len());
        let result = Record::from_buffer(&mut &buf[..]).unwrap();
        assert_eq!(result, Some(rec));

        buf.clear();
        let rec = Record::Error(ErrorRecord::BadRequest);
        let written = rec.write(Cursor::new(&mut buf)).await.unwrap();
        assert_eq!(written, buf.len());
        let result = Record::from_buffer(&mut &buf[..]).unwrap();
        assert_eq!(result, Some(rec));

        buf.clear();
        let rec = Record::SupportedMacAlgorithms(SupportedMacAlgorithms(vec![42]));
        let written = rec.write(Cursor::new(&mut buf)).await.unwrap();
        assert_eq!(written, buf.len());
        let result = Record::from_buffer(&mut &buf[..]).unwrap();
        assert_eq!(result, Some(rec));
    }

    #[tokio::test]
    async fn write_read_combined_records() {
        let mut buf = vec![];

        let rec = Record::CurrentParameters(ParameterSet {
            security_assocation: SecurityAssocation::from_key_data(
                11,
                &[1, 2, 3, 4, 5, 6, 7, 8, 9, 0],
            ),
            validity_period: ValidityPeriod {
                lifetime: 42,
                update_period: 43,
                grace_period: 44,
            },
        });
        let written = rec.write(Cursor::new(&mut buf)).await.unwrap();
        assert_eq!(written, buf.len());
        let result = Record::from_buffer(&mut &buf[..]).unwrap();
        assert_eq!(result, Some(rec));

        buf.clear();
        let rec = Record::NextParameters(ParameterSet {
            security_assocation: SecurityAssocation::from_key_data(
                11,
                &[1, 2, 3, 4, 5, 6, 7, 8, 9, 0],
            ),
            validity_period: ValidityPeriod {
                lifetime: 42,
                update_period: 43,
                grace_period: 44,
            },
        });
        let written = rec.write(Cursor::new(&mut buf)).await.unwrap();
        assert_eq!(written, buf.len());
        let result = Record::from_buffer(&mut &buf[..]).unwrap();
        assert_eq!(result, Some(rec));
    }

    #[tokio::test]
    async fn write_read_request() {
        let mut buf = vec![];
        let request = PtpKeyRequestMessage {
            next_protocol: NextProtocols::ptpv2_1(),
            association_mode: AssociationMode::Group {
                ptp_domain_number: 10,
                sdo_id: SdoId::try_from(21).unwrap(),
                subgroup: 16,
            },
        };
        let written = request.write(Cursor::new(&mut buf)).await.unwrap();
        assert_eq!(written, buf.len());

        let v = Record::read_until_eom(&mut &buf[..]).unwrap().unwrap();
        let result = PtpKeyRequestMessage::try_from(v).unwrap();
        assert_eq!(result, request);
    }

    #[tokio::test]
    async fn write_read_response() {
        let mut buf = vec![];
        let response = PtpKeyResponseMessage {
            next_protocol: NextProtocols::ptpv2_1(),
            current_parameters: ParameterSet {
                security_assocation: SecurityAssocation::from_key_data(
                    11,
                    &[1, 2, 3, 4, 5, 6, 7, 8, 9, 0],
                ),
                validity_period: ValidityPeriod {
                    lifetime: 42,
                    update_period: 43,
                    grace_period: 44,
                },
            },
            next_parameters: None,
        };
        let written = response.write(Cursor::new(&mut buf)).await.unwrap();
        assert_eq!(written, buf.len());

        let v = Record::read_until_eom(&mut &buf[..]).unwrap().unwrap();
        let result = PtpKeyResponseMessage::try_from(v).unwrap();
        assert_eq!(result, response);
    }
}
