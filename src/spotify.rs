use axum::body::Bytes;
use librespot_connect::{ConnectConfig, Spirc};
use librespot_core::{authentication::Credentials, Error, Session, SessionConfig};
use librespot_playback::{
    audio_backend::{Sink, SinkAsBytes, SinkResult, SinkError},
    config::{PlayerConfig},
    mixer,
    mixer::{NoOpVolume, MixerConfig},
    player::Player,
    decoder::AudioPacket,
    convert::Converter
};
use std::io::{self, Read};
use std::sync::mpsc as std_mpsc;

/// play spirc stream using provided token. Pipes back to discord module.
/// Returns the Spirc handle alongside the reader so callers can stop/disconnect the device later.
pub async fn play_stream(access_token: String) -> Result<(Spirc, ChannelReader), Error> {
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

    let config = ConnectConfig {
        name: "Discord".to_owned(),
        // gives us a cool icon I hope
        device_type: librespot_core::config::DeviceType::GameConsole,
        is_group: false,
        initial_volume: u16::MAX/2,
        disable_volume: false,
        volume_steps: 64
    };
    
    // runs the session
    let (spirc, spirc_task) = Spirc::new(
        config,
        session,
        credentials,
        player,
        mixer
    ).await?;

    spirc.activate()?;

    // this returns the spirc object and stream then the function ends and spirc lives in discord
    tokio::spawn(spirc_task);
    Ok((spirc, ChannelReader::new(rx)))
}

/// Custom audio_backend for allowing audio stream to be returned as a function output 
pub struct ParentSink {
    tx: std_mpsc::SyncSender<Bytes>
}

impl Sink for ParentSink {
    // songbird's RawAdapter expects interleaved little-endian f32 PCM, so emit f32 (not s16).
    fn write(&mut self, packet: AudioPacket, converter: &mut Converter) -> SinkResult<()> {
        let samples = packet.samples().map_err(|e| SinkError::OnWrite(e.to_string()))?;
        let bytes: Vec<u8> = converter
            .f64_to_f32(samples)
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
    leftover: Bytes,
    // DIAGNOSTIC: log the first read so we can see if songbird ever consumes this reader.
    read_seen: bool
}

impl ChannelReader {
    pub fn new(rx: std_mpsc::Receiver<Bytes>) -> Self {
        Self {rx: std::sync::Mutex::new(rx), leftover: Bytes::new(), read_seen: false}
    }
}

// DIAGNOSTIC: pinpoint exactly when the receiver dies relative to the Spotify Play command.
impl Drop for ChannelReader {
    fn drop(&mut self) {
        eprintln!("[diag] ChannelReader dropped (songbird released the audio stream)");
    }
}

impl Read for ChannelReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // TEMP DIAGNOSTIC: confirm songbird actually starts pulling from this reader.
        if !self.read_seen {
            self.read_seen = true;
            eprintln!("[diag] ChannelReader: first read() call (songbird is consuming the stream)");
        }
        if self.leftover.is_empty() {
            match self.rx.lock().unwrap().try_recv() {
                Ok(chunk) => self.leftover = chunk,
                // no audio yet (or a momentary gap): fill the buffer with silence and keep going.
                Err(std_mpsc::TryRecvError::Empty) => {
                    // f32 PCM silence is all-zero bytes, so a zeroed buffer is a valid silent frame.
                    buf.fill(0);
                    return Ok(buf.len());
                }
                // every sender is gone — the player has shut down, so the stream is truly over.
                Err(std_mpsc::TryRecvError::Disconnected) => return Ok(0),
            }
        }

        let n = std::cmp::min(buf.len(), self.leftover.len());
        buf[..n].copy_from_slice(&self.leftover[..n]);
        self.leftover = self.leftover.slice(n..);
        Ok(n)
    }
}