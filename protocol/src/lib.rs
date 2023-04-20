use std::{ffi::OsString, path::PathBuf};

use bytes::{Bytes, BytesMut};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Request {
    /// Allocate a new terminal and start `command`.
    Start {
        command: Vec<OsString>,
        env:     Vec<(OsString, OsString)>,
        pwd:     PathBuf,
        rows:    u16,
        cols:    u16,
    },

    ListProcesses,
    Quit,

    /// Restart the stopped foreground process in the given terminal.
    /// Has no effect if the process is not stopped. If `id` is None, resumes
    /// the last stopped terminal.
    Resume {
        id:          Option<u32>,
        /// Whether the server should sent over the outputs it has accumulated
        with_output: bool,
    },

    /// Window size has changed. Should be sent by the client in response to
    /// the SIGWINCH signal.
    WindowSize {
        id:   u32,
        rows: u16,
        cols: u16,
    },
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
pub enum ExitStatus {
    Exited(i32),
    Signaled(i32),
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    Running,
    Stopped,
    Terminated(ExitStatus),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Process {
    pub id:        u32,
    pub pid:       u32,
    pub state:     ProcessState,
    pub connected: bool,
    pub command:   OsString,
}

#[derive(Serialize, Deserialize, Debug, Clone, thiserror::Error)]
pub enum Error {
    #[error("Job {id:?} not found")]
    NotFound { id: Option<u32> },

    #[error("Job {id} is already foreground somewhere else")]
    AlreadyConnected { id: u32 },

    #[error("Client sent an invalid request")]
    InvalidRequest,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Event {
    /// Process in the given terminal changed state, i.e. exist, stopped,
    /// started, or killed. When this is sent in response to a Start
    /// request, `id` is a newly allocated identifier.
    StateChanged {
        /// The unique identifier for the terminal.
        id:    u32,
        state: ProcessState,
    },

    /// Sent on `Resume`, report the current terminal size
    WindowSize {
        cols: u16,
        rows: u16,
    },

    Error(Error),

    /// Responses for the ListProcesses request
    Process(Process),
}

pub use bytes;

pub struct MapCodec<T, EI, DO, R, W, FD, FE> {
    inner:      T,
    map_decode: FD,
    map_encode: FE,
    #[allow(clippy::type_complexity)]
    _marker:    std::marker::PhantomData<(fn(DO) -> R, fn(W) -> EI)>,
}

impl<T, EI, DO, R, W, FD, FE> MapCodec<T, EI, DO, R, W, FD, FE> {
    pub fn new(inner: T, map_decode: FD, map_encode: FE) -> Self {
        Self {
            inner,
            map_decode,
            map_encode,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<T, EI, DO, R, W, FD: FnMut(DO) -> R, FE> tokio_util::codec::Decoder
    for MapCodec<T, EI, DO, R, W, FD, FE>
where
    T: tokio_util::codec::Decoder<Item = DO>,
{
    type Error = T::Error;
    type Item = R;

    fn decode(&mut self, src: &mut bytes::BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        Ok(self.inner.decode(src)?.map(|item| (self.map_decode)(item)))
    }
}

impl<T, EI, DO, R, W, FD, FE: FnMut(W) -> EI> tokio_util::codec::Encoder<W>
    for MapCodec<T, EI, DO, R, W, FD, FE>
where
    T: tokio_util::codec::Encoder<EI>,
{
    type Error = T::Error;

    fn encode(&mut self, item: W, dst: &mut bytes::BytesMut) -> Result<(), Self::Error> {
        self.inner.encode((self.map_encode)(item), dst)
    }
}
pub fn client_codec() -> impl tokio_util::codec::Encoder<Request, Error = std::io::Error>
       + tokio_util::codec::Decoder<Item = Event, Error = std::io::Error> {
    let codec = tokio_util::codec::length_delimited::LengthDelimitedCodec::new();
    MapCodec::new(
        codec,
        |bytes: BytesMut| bincode::deserialize(&bytes).unwrap(),
        |event| -> Bytes { bincode::serialize(&event).unwrap().into() },
    )
}

pub fn server_codec() -> impl tokio_util::codec::Encoder<Event, Error = std::io::Error>
       + tokio_util::codec::Decoder<Item = Request, Error = std::io::Error> {
    let codec = tokio_util::codec::length_delimited::LengthDelimitedCodec::new();
    MapCodec::new(
        codec,
        |bytes: BytesMut| bincode::deserialize(&bytes).unwrap(),
        |request| -> Bytes { bincode::serialize(&request).unwrap().into() },
    )
}

pub trait Codec<Input, Output, Error> {
    fn as_encoder(&mut self) -> &mut dyn tokio_util::codec::Encoder<Input, Error = Error>;
    fn as_decoder(&mut self) -> &mut dyn tokio_util::codec::Decoder<Item = Output, Error = Error>;
}

impl<T, I, O, E> Codec<I, O, E> for T
where
    T: tokio_util::codec::Encoder<I, Error = E> + tokio_util::codec::Decoder<Item = O, Error = E>,
{
    fn as_encoder(&mut self) -> &mut dyn tokio_util::codec::Encoder<I, Error = E> {
        self as _
    }

    fn as_decoder(&mut self) -> &mut dyn tokio_util::codec::Decoder<Item = O, Error = E> {
        self as _
    }
}

impl<I, O, E: From<std::io::Error>> tokio_util::codec::Encoder<I>
    for Box<dyn Codec<I, O, E> + Send + Sync + Unpin>
{
    type Error = E;

    fn encode(&mut self, item: I, dst: &mut bytes::BytesMut) -> Result<(), Self::Error> {
        self.as_mut().as_encoder().encode(item, dst)
    }
}

impl<I, O, E: From<std::io::Error>> tokio_util::codec::Decoder
    for Box<dyn Codec<I, O, E> + Send + Sync + Unpin>
{
    type Error = E;
    type Item = O;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        self.as_mut().as_decoder().decode(src)
    }

    fn decode_eof(&mut self, buf: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        self.as_mut().as_decoder().decode_eof(buf)
    }
}

pub type DynServerCodec = Box<dyn Codec<Event, Request, std::io::Error> + Send + Sync + Unpin>;
pub type DynClientCodec = Box<dyn Codec<Request, Event, std::io::Error> + Send + Sync + Unpin>;
