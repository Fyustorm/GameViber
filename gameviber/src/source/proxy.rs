//! Source « proxy » : la vraie manette est grab et une copie virtuelle
//! (uinput, même nom et même VID/PID) est exposée aux jeux. Les inputs sont
//! relayés vers la copie ; le force-feedback que le jeu y téléverse est
//! capturé puis relayé à la vraie manette (passthrough).

use std::collections::HashMap;
use std::io;
use std::os::fd::AsRawFd;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use anyhow::Context;
use evdev::uinput::VirtualDevice;
use evdev::{
    AbsInfo, AttributeSet, Device, EventSummary, EventType, FFEffect, FFEffectCode, InputEvent,
    SynchronizationCode, UInputCode, UinputAbsSetup,
};

use super::{find_gamepad, translate_input, EventSender, SourceEvent, SourceKind};
use crate::rumble::Effect;

const FF_CODES: [FFEffectCode; 6] = [
    FFEffectCode::FF_RUMBLE,
    FFEffectCode::FF_PERIODIC,
    FFEffectCode::FF_SQUARE,
    FFEffectCode::FF_TRIANGLE,
    FFEffectCode::FF_SINE,
    FFEffectCode::FF_GAIN,
];

pub struct ProxySource {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    hider: Option<DeviceHider>,
}

impl ProxySource {
    pub fn start(device: Option<&Path>, passthrough: bool, hide: bool, tx: EventSender) -> anyhow::Result<Self> {
        let (real_path, mut real) = find_gamepad(device)?;
        let mut virt = build_virtual(&real).context("création de la manette virtuelle")?;
        let virt_path = virt
            .enumerate_dev_nodes_blocking()
            .context("recherche du nœud de la manette virtuelle")?
            .filter_map(Result::ok)
            .find(|p| p.to_string_lossy().contains("event"))
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| "?".into());
        real.grab().context("grab de la vraie manette")?;
        log::info!(
            "Manette virtuelle '{}' créée sur {virt_path} (réelle {real_path} grab)",
            real.name().unwrap_or("?")
        );

        let hider = if hide {
            let h = DeviceHider::hide(&real_path)?;
            Some(h)
        } else {
            None
        };

        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let stop = stop.clone();
            std::thread::Builder::new().name("proxy".into()).spawn(move || {
                let mut proxy = Proxy { real, virt, real_effects: HashMap::new(), passthrough, tx, device: virt_path };
                if let Err(e) = proxy.run(&stop) {
                    log::error!("proxy arrêté : {e:#}");
                }
            })?
        };
        Ok(Self { stop, thread: Some(thread), hider })
    }

    pub fn shutdown(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        if let Some(h) = self.hider.take() {
            h.restore();
        }
    }
}

fn build_virtual(real: &Device) -> anyhow::Result<VirtualDevice> {
    let name = real.name().unwrap_or("Gamepad").to_owned();
    let mut builder = VirtualDevice::builder()?
        .name(&name)
        .input_id(real.input_id())
        // Pas de with_phys : evdev 0.13 encode mal UI_SET_PHYS (taille 1 au lieu
        // d'un pointeur) et le noyau le refuse. Aucun jeu ne lit le phys.
        .with_ff(&FF_CODES.iter().copied().collect::<AttributeSet<_>>())
        .context("ff")?
        .with_ff_effects_max(real.max_ff_effects().max(1) as u32);
    if let Some(keys) = real.supported_keys() {
        builder = builder.with_keys(keys).context("keys")?;
    }
    if let Some(axes) = real.supported_absolute_axes() {
        let state = real.get_abs_state()?;
        for axis in axes.iter() {
            let a = state[axis.0 as usize];
            let info = AbsInfo::new(a.value, a.minimum, a.maximum, a.fuzz, a.flat, a.resolution);
            builder = builder
                .with_absolute_axis(&UinputAbsSetup::new(axis, info))
                .with_context(|| format!("axe {axis:?}"))?;
        }
    }
    builder.build().context("UI_DEV_CREATE")
}

struct Proxy {
    real: Device,
    virt: VirtualDevice,
    /// id de l'effet côté manette virtuelle -> effet téléversé sur la vraie.
    real_effects: HashMap<i16, FFEffect>,
    passthrough: bool,
    tx: EventSender,
    device: String,
}

impl Proxy {
    fn run(&mut self, stop: &AtomicBool) -> anyhow::Result<()> {
        self.real.set_nonblocking(true)?;
        set_nonblocking(self.virt.as_raw_fd())?;
        let mut pending: Vec<InputEvent> = Vec::new();
        while !stop.load(Ordering::Relaxed) {
            let mut fds = [
                libc::pollfd { fd: self.real.as_raw_fd(), events: libc::POLLIN, revents: 0 },
                libc::pollfd { fd: self.virt.as_raw_fd(), events: libc::POLLIN, revents: 0 },
            ];
            if unsafe { libc::poll(fds.as_mut_ptr(), 2, 200) } < 0 {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(err.into());
            }
            if fds[0].revents & (libc::POLLERR | libc::POLLHUP) != 0 {
                anyhow::bail!("manette réelle déconnectée");
            }
            if fds[0].revents & libc::POLLIN != 0 {
                self.forward_inputs(&mut pending)?;
            }
            if fds[1].revents & libc::POLLIN != 0 {
                self.handle_game_ff()?;
            }
        }
        Ok(())
    }

    /// Vraie manette -> manette virtuelle, trame par trame (SYN_REPORT).
    fn forward_inputs(&mut self, pending: &mut Vec<InputEvent>) -> anyhow::Result<()> {
        let events: Vec<InputEvent> = match self.real.fetch_events() {
            Ok(it) => it.collect(),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        for ev in events {
            match ev.event_type() {
                EventType::SYNCHRONIZATION if ev.code() == SynchronizationCode::SYN_REPORT.0 => {
                    self.virt.emit(pending)?; // ajoute lui-même le SYN_REPORT
                    pending.clear();
                }
                EventType::SYNCHRONIZATION | EventType::FORCEFEEDBACK => {}
                _ => {
                    pending.push(ev);
                    if let Some(kind @ (SourceKind::Button { .. } | SourceKind::Axis { .. })) = translate_input(ev) {
                        self.send(kind);
                    }
                }
            }
        }
        Ok(())
    }

    /// Jeu -> manette virtuelle : uploads, effacements, play/stop, gain.
    fn handle_game_ff(&mut self) -> anyhow::Result<()> {
        let events: Vec<InputEvent> = match self.virt.fetch_events() {
            Ok(it) => it.collect(),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        for ev in events {
            match ev.destructure() {
                EventSummary::UInput(ev, UInputCode::UI_FF_UPLOAD, _) => {
                    let mut upload = self.virt.process_ff_upload(ev)?;
                    let id = upload.effect_id();
                    let data = upload.effect();
                    if self.passthrough {
                        let result = match self.real_effects.get_mut(&id) {
                            Some(effect) => effect.update(data),
                            None => self.real.upload_ff_effect(data).map(|e| {
                                self.real_effects.insert(id, e);
                            }),
                        };
                        if let Err(e) = result {
                            log::warn!("upload sur la vraie manette échoué : {e}");
                        }
                    }
                    upload.set_retval(0);
                    drop(upload); // UI_END_FF_UPLOAD : débloque le jeu
                    self.send(SourceKind::Upload { id, effect: Effect::from_evdev(&data) });
                }
                EventSummary::UInput(ev, UInputCode::UI_FF_ERASE, _) => {
                    let erase = self.virt.process_ff_erase(ev)?;
                    let id = erase.effect_id() as i16;
                    drop(erase);
                    self.real_effects.remove(&id); // Drop de FFEffect = effacement
                    self.send(SourceKind::Erase { id });
                }
                _ => {
                    let Some(kind) = translate_input(ev) else { continue };
                    if self.passthrough {
                        self.passthrough_ff(&kind);
                    }
                    self.send(kind);
                }
            }
        }
        Ok(())
    }

    fn passthrough_ff(&mut self, kind: &SourceKind) {
        let result = match *kind {
            SourceKind::Play { id, count } => match self.real_effects.get_mut(&id) {
                Some(effect) if count > 0 => effect.play(count),
                Some(effect) => effect.stop(),
                None => Ok(()),
            },
            SourceKind::Gain(gain) => self.real.send_events(&[InputEvent::new(
                EventType::FORCEFEEDBACK.0,
                FFEffectCode::FF_GAIN.0,
                gain as i32,
            )]),
            _ => Ok(()),
        };
        if let Err(e) = result {
            log::warn!("passthrough FF échoué : {e}");
        }
    }

    fn send(&self, kind: SourceKind) {
        let _ = self.tx.send(SourceEvent { device: self.device.clone(), kind });
    }
}

fn set_nonblocking(fd: i32) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Rend la vraie manette invisible aux jeux (root requis) : chmod 0600 et
/// suppression des ACL uaccess de ses nœuds eventX / jsX, restaurés à la sortie.
struct DeviceHider {
    saved: Vec<(String, u32, String)>, // (nœud, mode, sortie getfacl)
}

impl DeviceHider {
    fn hide(event_path: &str) -> anyhow::Result<Self> {
        use std::os::unix::fs::PermissionsExt;
        anyhow::ensure!(unsafe { libc::geteuid() } == 0, "--hide nécessite root (sudo)");
        let event = Path::new(event_path).file_name().context("chemin de manette invalide")?;
        let sysdir = Path::new("/sys/class/input").join(event).join("device");
        let mut me = Self { saved: Vec::new() };
        for entry in std::fs::read_dir(&sysdir)? {
            let name = entry?.file_name().to_string_lossy().into_owned();
            if !(name.starts_with("event") || name.starts_with("js")) {
                continue;
            }
            let node = format!("/dev/input/{name}");
            let mode = std::fs::metadata(&node)?.permissions().mode() & 0o7777;
            let acl = Command::new("getfacl").args(["-p", &node]).output()?;
            anyhow::ensure!(acl.status.success(), "getfacl {node} a échoué");
            me.saved.push((node.clone(), mode, String::from_utf8_lossy(&acl.stdout).into_owned()));
            anyhow::ensure!(Command::new("setfacl").args(["-b", &node]).status()?.success(), "setfacl -b {node} a échoué");
            std::fs::set_permissions(&node, std::fs::Permissions::from_mode(0o600))?;
            log::info!("Manette réelle masquée : {node}");
        }
        Ok(me)
    }

    fn restore(self) {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        for (node, mode, acl) in self.saved {
            if let Err(e) = std::fs::set_permissions(&node, std::fs::Permissions::from_mode(mode)) {
                log::warn!("restauration des droits de {node} échouée : {e}");
            }
            let child = Command::new("setfacl").arg("--restore=-").stdin(std::process::Stdio::piped()).spawn();
            match child {
                Ok(mut child) => {
                    if let Some(mut stdin) = child.stdin.take() {
                        let _ = stdin.write_all(acl.as_bytes());
                    }
                    let _ = child.wait();
                }
                Err(e) => log::warn!("restauration des ACL de {node} échouée : {e}"),
            }
        }
    }
}
