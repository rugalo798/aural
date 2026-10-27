use anyhow::{Context, Result, anyhow};
use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Player};
use std::fs::File;
use std::io::Cursor;
use std::path::Path;
use std::time::Duration;

pub struct AudioEngine {
    _device: MixerDeviceSink,
    player: Player,
}

impl AudioEngine {
    pub fn new() -> Result<Self> {
        let mut device = DeviceSinkBuilder::open_default_sink()
            .map_err(|e| anyhow!("abrindo dispositivo de áudio: {e}"))?;
        device.log_on_drop(false);
        let player = Player::connect_new(device.mixer());
        Ok(Self {
            _device: device,
            player,
        })
    }

    pub fn play_path(&self, path: &Path) -> Result<()> {
        self.player.stop();
        let file = File::open(path).with_context(|| format!("abrindo {}", path.display()))?;
        let decoder = Decoder::try_from(file).context("decodificando áudio")?;
        self.player.append(decoder);
        self.player.play();
        Ok(())
    }

    pub fn play_bytes(&self, bytes: Vec<u8>) -> Result<()> {
        self.player.stop();
        let decoder = Decoder::try_from(Cursor::new(bytes)).context("decodificando stream")?;
        self.player.append(decoder);
        self.player.play();
        Ok(())
    }

    pub fn pause(&self) {
        self.player.pause();
    }

    pub fn resume(&self) {
        self.player.play();
    }

    pub fn stop(&self) {
        self.player.stop();
    }

    pub fn seek(&self, secs: f64) -> Result<()> {
        let secs = secs.max(0.0);
        self.player
            .try_seek(Duration::from_secs_f64(secs))
            .map_err(|e| anyhow!("seek: {e}"))
    }

    pub fn set_volume(&self, volume: f32) {
        self.player.set_volume(volume.clamp(0.0, 1.0));
    }

    pub fn position(&self) -> Duration {
        self.player.get_pos()
    }

    pub fn is_empty(&self) -> bool {
        self.player.empty()
    }

    pub fn is_paused(&self) -> bool {
        self.player.is_paused()
    }
}
