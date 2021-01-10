// Copyright 2020 - developers of the `grammers` project.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.
mod generated;

pub use generated::LAYER as VERSION;
use generated::{enums, types};
use grammers_crypto::auth_key::AuthKey;
use grammers_tl_types::deserialize::Error as DeserializeError;
use log::warn;
use std::collections::HashMap;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, Write};
use std::net::Ipv4Addr;
use std::path::Path;

// Needed for auto-generated definitions.
use grammers_tl_types::{deserialize, serialize, Deserializable, Identifiable, Serializable};

pub struct UpdateState {
    pub pts: i32,
    pub qts: i32,
    pub date: i32,
    pub seq: i32,
    pub channels: HashMap<i32, i32>,
}

pub struct MemorySession {
    session: types::Session,
}

pub struct FileSession {
    file: File,
    session: MemorySession,
}

#[derive(Debug)]
pub enum Error {
    MalformedData,
    UnsupportedVersion,
}

pub trait Session {
    /// User's home datacenter ID, if known.
    fn user_dc(&self) -> Option<i32>;

    /// Authorization key data for the given datacenter ID, if any.
    fn dc_auth_key(&self, dc_id: i32) -> Option<AuthKey>;

    fn insert_dc(&mut self, id: i32, ipv4_port: (Ipv4Addr, u16), auth: &AuthKey);

    fn set_user(&mut self, id: i32, dc: i32, bot: bool);

    fn set_state(&mut self, state: UpdateState);
}

impl Session for MemorySession {
    fn user_dc(&self) -> Option<i32> {
        self.session
            .user
            .as_ref()
            .map(|enums::User::User(user)| user.dc)
    }

    fn dc_auth_key(&self, dc_id: i32) -> Option<AuthKey> {
        self.session
            .dcs
            .iter()
            .filter_map(|enums::DataCenter::Center(dc)| {
                if dc.id == dc_id {
                    if let Some(auth) = &dc.auth {
                        let mut bytes = [0; 256];
                        bytes.copy_from_slice(auth);
                        Some(AuthKey::from_bytes(bytes))
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .next()
    }

    fn insert_dc(&mut self, id: i32, (ipv4, port): (Ipv4Addr, u16), auth: &AuthKey) {
        if let Some(pos) = self
            .session
            .dcs
            .iter()
            .position(|enums::DataCenter::Center(dc)| dc.id == id)
        {
            self.session.dcs.remove(pos);
        }
        self.session.dcs.push(
            types::DataCenter {
                id,
                ipv4: Some(i32::from_le_bytes(ipv4.octets())),
                ipv6: None,
                port: port as i32,
                auth: Some(auth.to_bytes().to_vec()),
            }
            .into(),
        );
    }

    fn set_user(&mut self, id: i32, dc: i32, bot: bool) {
        self.session.user = Some(types::User { id, dc, bot }.into())
    }

    fn set_state(&mut self, state: UpdateState) {
        self.session.state = Some(
            types::UpdateState {
                pts: state.pts,
                qts: state.qts,
                date: state.date,
                seq: state.seq,
                channels: state
                    .channels
                    .into_iter()
                    .map(|(channel_id, pts)| types::ChannelState { channel_id, pts }.into())
                    .collect(),
            }
            .into(),
        )
    }
}

impl Session for FileSession {
    fn user_dc(&self) -> Option<i32> {
        self.session.user_dc()
    }

    fn dc_auth_key(&self, dc_id: i32) -> Option<AuthKey> {
        self.session.dc_auth_key(dc_id)
    }

    fn insert_dc(&mut self, id: i32, ipv4_port: (Ipv4Addr, u16), auth: &AuthKey) {
        self.session.insert_dc(id, ipv4_port, auth)
    }

    fn set_user(&mut self, id: i32, dc: i32, bot: bool) {
        self.session.set_user(id, dc, bot)
    }

    fn set_state(&mut self, state: UpdateState) {
        self.session.set_state(state)
    }
}

impl MemorySession {
    pub fn new() -> Self {
        Self {
            session: types::Session {
                dcs: Vec::new(),
                user: None,
                state: None,
            },
        }
    }

    pub fn load(data: &[u8]) -> Result<Self, Error> {
        Ok(Self {
            session: enums::Session::from_bytes(&data)
                .map_err(|e| match e {
                    DeserializeError::UnexpectedEof => Error::MalformedData,
                    DeserializeError::UnexpectedConstructor { .. } => Error::UnsupportedVersion,
                })?
                .into(),
        })
    }

    pub fn save(&self) -> Vec<u8> {
        enums::Session::Session(self.session.clone()).to_bytes()
    }
}

impl FileSession {
    /// Loads or creates a new session file.
    pub fn load_or_create<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        if let Ok(instance) = Self::load(path.as_ref()) {
            Ok(instance)
        } else {
            Self::create(path)
        }
    }

    /// Create a new session instance.
    pub fn create<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let mut this = Self {
            file: File::create(path)?,
            session: MemorySession::new(),
        };
        // Immediately save or else we'll have an empty (and invalid) session file.
        this.save()?;
        Ok(this)
    }

    /// Load a previous session instance.
    pub fn load<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let mut data = Vec::new();
        File::open(path.as_ref())?.read_to_end(&mut data)?;
        let session = MemorySession::load(&data)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        Ok(Self {
            file: OpenOptions::new().write(true).open(path.as_ref())?,
            session,
        })
    }

    /// Saves the session file.
    pub fn save(&mut self) -> io::Result<()> {
        self.file.seek(io::SeekFrom::Start(0))?;
        self.file.set_len(0)?;
        self.file.write_all(&self.session.save())?;
        self.file.sync_data()?;
        Ok(())
    }
}

impl Drop for FileSession {
    fn drop(&mut self) {
        match self.save() {
            Ok(_) => {}
            Err(e) => warn!("failed to save session on drop: {}", e),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::MalformedData => write!(f, "malformed data"),
            Error::UnsupportedVersion => write!(f, "unsupported version"),
        }
    }
}

impl std::error::Error for Error {}
