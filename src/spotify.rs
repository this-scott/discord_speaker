use axum::body::Bytes;
use librespot_connect::{ConnectConfig, Spirc};
use librespot_core::{authentication::Credentials, Error, Session, SessionConfig};
use librespot_playback::{
    audio_backend::{Sink, SinkAsBytes, SinkResult, SinkError},
    config::{AudioFormat, PlayerConfig},
    mixer,
    mixer::{NoOpVolume, MixerConfig},
    player::Player,
    decoder::AudioPacket,
    convert::Converter
};
use std::io::{self, Read};
use std::sync::mpsc as std_mpsc;
use tokio_util::sync::CancellationToken;

// reminder: cancellation token
/// play spirc stream using provided token. Pipes back to discord module
pub async fn play_stream(access_token: String, cancel_token: CancellationToken) -> Result<ChannelReader, Error> {
    //stream channel
    let (tx, rx) = std_mpsc::sync_channel::<Bytes>(64);
    
    let credentials = Credentials::with_access_token(access_token);
    let session = Session::new(SessionConfig::default(), None);

    // Need a custom backend if I don't want to spawn this as a subprocess.
    // backend pumps the stream as an object we can return instead of calling to a system or returning as stdout
    // Librespot needs docs... 
    let player = Player::new(
        PlayerConfig::default(),
        session.clone(),
        Box::new(NoOpVolume),
        move || Box::new(ParentSink::new(tx.clone())),
    );

    let mixer_builder = mixer::find(None).unwrap();
    let mixer = mixer_builder(MixerConfig::default())?;

    // runs the session
    let (_, spirc_task) = Spirc::new(
        // ConnectConfig is where we name the session. Rebuild custom later
        ConnectConfig::default(),
        session,
        credentials,
        player,
        mixer
    ).await?;
    tokio::spawn(spirc_task);
    Ok(ChannelReader::new(rx))
}

/// Custom audio_backend for allowing audio stream to be returned as a function output 
pub struct ParentSink {
    tx: std_mpsc::SyncSender<Bytes>
}

impl Sink for ParentSink {
    // writes the stream to unsigned 8 byte pcm packets
    fn write(&mut self, packet: AudioPacket, converter: &mut Converter) -> SinkResult<()> {
        let samples = packet.samples().map_err(|e| SinkError::OnWrite(e.to_string()))?;
        let bytes: Vec<u8> = converter
            .f64_to_s16(samples)
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        self.write_bytes(&bytes)
    }
}

impl SinkAsBytes for ParentSink {
    fn write_bytes(&mut self, data: &[u8]) -> SinkResult<()> {
        // using blocking send bc channel stream creation is synchronous but receiver is async
        self.tx.send(Bytes::copy_from_slice(data)).map_err(|_| SinkError::ConnectionRefused("receiver dropped".into()))
    }
}

impl ParentSink {
    pub fn new(tx: std_mpsc::SyncSender<Bytes>) -> Self {
        Self {tx}
    }
}

/// Wrapping a std async mpsc receiver into std::io::Read to feed into mss. 
pub struct ChannelReader {
    rx: std::sync::Mutex<std_mpsc::Receiver<Bytes>>,
    leftover: Bytes
}

impl ChannelReader {
    pub fn new(rx: std_mpsc::Receiver<Bytes>) -> Self {
        Self {rx: std::sync::Mutex::new(rx), leftover: Bytes::new()}
    }
}

impl Read for ChannelReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.leftover.is_empty() {
            match self.rx.lock().unwrap().recv() {
                Ok(chunk) => self.leftover = chunk,
                Err(_) => return Ok(0) //end of file
            }
        }

        let n = std::cmp::min(buf.len(), self.leftover.len());
        buf[..n].copy_from_slice(&self.leftover[..n]);
        self.leftover = self.leftover.slice(n..);
        Ok(n)
    }
}