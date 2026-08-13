//! Play, loop, stop: the whole transport a promo loop needs.

use std::path::Path;

use drm::control::crtc;

use denise::Rect;
use denise_drm::Card;

use crate::annexb::{AccessUnits, Codec};
use crate::decode::Decoder;
use crate::plane::VideoPlane;
use crate::{Asset, Decoders, VideoError};

/// An elementary stream playing onto a plane.
///
/// Driven from the application's own event loop: call [`Player::pump`] each
/// pass; it feeds the decoder what fits, flips a frame when one is ready, and
/// never blocks. There is no seeking — restart-from-start is the only seek a
/// promo loop has, and [looping](Player::set_looping) does it by itself.
pub struct Player {
    stream: Vec<u8>,
    codec: Codec,
    decoder: Decoder,
    plane: VideoPlane,
    /// Byte offset ranges of each access unit, precomputed once.
    units: Vec<(usize, usize)>,
    /// The next unit to feed.
    cursor: usize,
    looping: bool,
    /// The frame currently on screen, returned to the decoder when the next
    /// one replaces it.
    on_screen: Option<u32>,
    frames_shown: u64,
}

impl Player {
    /// Opens the first of `assets` this board's hardware plays, positioned at
    /// `dst` on the surface's CRTC.
    ///
    /// `card` and `crtc` come from the surface —
    /// [`DrmSurface::card`](denise_drm::DrmSurface::card) and
    /// [`DrmSurface::crtc`](denise_drm::DrmSurface::crtc) — because one
    /// process is DRM master and the plane must be driven through the same
    /// device, never a second open.
    pub fn open(
        assets: &[Asset],
        card: &Card,
        crtc: crtc::Handle,
        dst: Rect,
    ) -> Result<Self, VideoError> {
        let decoders = Decoders::detect();
        let (asset, node) = decoders.pick(assets).ok_or(VideoError::NothingPlayable)?;
        let stream = std::fs::read(&asset.path).map_err(|source| VideoError::Open {
            path: asset.path.clone(),
            source,
        })?;
        Self::from_stream(stream, asset.codec, &node.path, card, crtc, dst)
    }

    /// [`Player::open`] with the stream bytes and decoder node already chosen —
    /// what the `player` example uses to honour an explicit device argument.
    pub fn from_stream(
        stream: Vec<u8>,
        codec: Codec,
        node: impl AsRef<Path>,
        card: &Card,
        crtc: crtc::Handle,
        dst: Rect,
    ) -> Result<Self, VideoError> {
        let units: Vec<(usize, usize)> = {
            let mut ranges = Vec::new();
            let base = stream.as_ptr() as usize;
            for unit in AccessUnits::new(&stream, codec) {
                let start = unit.as_ptr() as usize - base;
                ranges.push((start, start + unit.len()));
            }
            ranges
        };
        if units.is_empty() {
            return Err(VideoError::NoFrames);
        }
        let decoder = Decoder::open(node, codec)?;
        let plane = VideoPlane::new(card, crtc, dst)?;
        Ok(Self {
            stream,
            codec,
            decoder,
            plane,
            units,
            cursor: 0,
            looping: true,
            on_screen: None,
            frames_shown: 0,
        })
    }

    /// Which codec is playing.
    pub fn codec(&self) -> Codec {
        self.codec
    }

    /// Frames flipped onto the plane so far.
    pub fn frames_shown(&self) -> u64 {
        self.frames_shown
    }

    /// Whether reaching the end starts over. On by default; off, the last
    /// frame holds on screen.
    pub fn set_looping(&mut self, looping: bool) {
        self.looping = looping;
    }

    /// Whether the stream has been fed to its end and looping is off.
    pub fn finished(&self) -> bool {
        !self.looping && self.cursor >= self.units.len()
    }

    /// Moves where the video sits, for a tree that relaid out.
    pub fn set_dst(&mut self, dst: Rect) {
        self.plane.set_dst(dst);
    }

    /// One pass of the transport: feed what fits, show what is ready.
    ///
    /// Returns `true` if a new frame went on screen — the caller needs no
    /// repaint either way, the plane is composited by the display controller.
    pub fn pump(&mut self, card: &Card) -> Result<bool, VideoError> {
        // Feed as long as the decoder has room and the stream has units.
        while self.cursor < self.units.len() && self.decoder.ready_for_input() {
            let (start, end) = self.units[self.cursor];
            if self.decoder.feed(&self.stream[start..end])? {
                self.cursor += 1;
            } else {
                break;
            }
        }
        if self.cursor >= self.units.len() && self.looping {
            // The only seek there is: back to the parameter sets.
            self.cursor = 0;
        }

        match self.decoder.pump()? {
            None => Ok(false),
            Some(frame) => {
                self.plane.show(card, &self.decoder, &frame)?;
                self.frames_shown += 1;
                // The previous frame has left the screen with this flip; the
                // decoder may fill it again.
                if let Some(previous) = self.on_screen.replace(frame.index) {
                    self.decoder.recycle(previous)?;
                }
                Ok(true)
            }
        }
    }

    /// Takes the plane off screen and rewinds. [`Player::pump`] starts over.
    pub fn stop(&mut self, card: &Card) -> Result<(), VideoError> {
        self.plane.hide(card)?;
        self.decoder.restart()?;
        self.cursor = 0;
        self.on_screen = None;
        Ok(())
    }
}
