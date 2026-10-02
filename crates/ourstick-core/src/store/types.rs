//! The values a [`Store`](super::Store) keeps.

use std::path::PathBuf;

use crate::cipher::CipherId;
use crate::manifest::FileStamp;
use crate::record::SignedManifestRecord;
use crate::stick_id::StickId;
use crate::ticket::NodeId;

/// Where this device keeps its secrets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretBackend {
    /// The freedesktop Secret Service (Linux).
    SecretService,
    /// The login keychain (macOS).
    Keychain,
    /// The user's Credential Manager (Windows).
    CredentialManager,
    /// A key file the user chose.
    KeyFile(PathBuf),
}

/// This device. Created once, never replaced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    /// The device's own node key, shared by the sticks made before ADR 0009;
    /// `None` on a device made after it, whose sticks each have their own.
    pub node_id: Option<NodeId>,
    pub secret_backend: SecretBackend,
    pub created_at_ms: u64,
}

/// Which program lists a stick (ADR 0009).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StickKind {
    /// The app's: a drive, whose folder carries the drive marker and has
    /// no fixed size (ADR 0008).
    App,
    /// Shared from the command line.
    Cli,
}

/// A stick this device owns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedStick {
    pub stick_id: StickId,
    pub name: String,
    pub root_path: PathBuf,
    pub cipher: CipherId,
    /// Highest `seq` ever signed; 0 if none.
    pub last_signed_seq: u64,
    /// The latest signed record, if any. Its `seq` equals `last_signed_seq`.
    pub latest_record: Option<SignedManifestRecord>,
    /// Whether `latest_record` is of a partial manifest (ADR 0007): no
    /// complete version was published since.
    pub latest_partial: bool,
    pub created_at_ms: u64,
    /// Windows: the drive letter the stick was last given (`A`–`Z`), asked
    /// for again after a restart. `None` elsewhere.
    pub drive_letter: Option<char>,
    /// The node id of the stick's own node key (ADR 0009); `None` for a
    /// stick made before, which uses the device's.
    pub node_id: Option<NodeId>,
    pub kind: StickKind,
    /// macOS: the app stick's disk image, recorded once, never derived from
    /// its name or id.
    pub image_path: Option<PathBuf>,
    /// False after the user's "Stop sharing", so a restart keeps it stopped.
    pub sharing_wanted: bool,
}

impl OwnedStick {
    /// A stick that has not signed anything yet.
    pub fn new(
        stick_id: StickId,
        name: impl Into<String>,
        root_path: impl Into<PathBuf>,
        cipher: CipherId,
        created_at_ms: u64,
    ) -> Self {
        Self {
            stick_id,
            name: name.into(),
            root_path: root_path.into(),
            cipher,
            last_signed_seq: 0,
            latest_record: None,
            latest_partial: false,
            created_at_ms,
            drive_letter: None,
            node_id: None,
            kind: StickKind::Cli,
            image_path: None,
            sharing_wanted: true,
        }
    }
}

/// One version of a stick's manifest: the `seq` and manifest hash of a signed
/// record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ManifestVersion {
    pub seq: u64,
    pub hash: [u8; 32],
}

/// A stick this device reads, and what its folder holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinedStick {
    pub stick_id: StickId,
    /// The folder of the latest `get`; another folder replaces it.
    pub output_dir: PathBuf,
    /// The version whose files are all in `output_dir`; `None` until a
    /// download finishes there.
    pub completed: Option<ManifestVersion>,
    pub joined_at_ms: u64,
    /// The program that receives it: `get` (`Cli`) or the receiver window
    /// (`App`), which `get` never moves to another folder.
    pub kind: StickKind,
    /// Its name as the last verified manifest gave it; `None` before one.
    pub name: Option<String>,
    /// The user paused it (the receiver window).
    pub paused: bool,
}

impl JoinedStick {
    /// A stick joined from a ticket by `get`, before any record was
    /// accepted.
    pub fn new(stick_id: StickId, output_dir: impl Into<PathBuf>, joined_at_ms: u64) -> Self {
        Self {
            stick_id,
            output_dir: output_dir.into(),
            completed: None,
            joined_at_ms,
            kind: StickKind::Cli,
            name: None,
            paused: false,
        }
    }
}

/// A change to a received stick's row ([`crate::store::Store::change_joined_stick`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JoinedChange {
    /// Its name, from a verified manifest.
    Name(String),
    /// The program that receives it from now on.
    Kind(StickKind),
    Paused(bool),
    /// Being forgotten: hidden from then on, finished by
    /// `received::forget_received` or at the next start.
    Forgetting,
}

/// What the owner last encrypted for one file of a stick: the file's stamp
/// then, and the blob made from it. Lets an unchanged file skip encryption.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedFile {
    /// Path relative to the stick root, as in the manifest.
    pub path: String,
    pub stamp: FileStamp,
    pub blob_hash: [u8; 32],
    /// BLAKE3 of the file's plaintext then (the nonce prefix, ADR 0003); a
    /// hint only. `None` for files recorded before ADR 0007.
    pub plaintext_hash: Option<[u8; 32]>,
}

/// A file `get` wrote into a joined stick's folder: the blob it came from,
/// and the file's stamp right after it was written. A file whose stamp is no
/// longer this was changed locally and is not overwritten.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadedFile {
    /// Its path in the stick, as in the manifest.
    pub path: String,
    /// Where it was saved, relative to the folder: `path`, or the smallest
    /// change of it this computer can save (ADR 0010).
    pub saved_as: String,
    pub stamp: FileStamp,
    pub blob_hash: [u8; 32],
}

/// A folder or file shared where it is, not copied onto its stick (ADR 0008):
/// a top-level entry of the stick under `name`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedEntry {
    /// Its name on the stick: the first component of its manifest paths.
    pub name: String,
    /// Where it is on this device. Never leaves the device.
    pub path: PathBuf,
    pub is_dir: bool,
    /// The disk it was on when linked (`st_dev` on Unix), if known.
    pub disk: Option<u64>,
    pub added_at_ms: u64,
}

/// What a claim on a disk's room is for (the space ledger).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClaimKind {
    /// A sharer's cache budget (ADR 0007), held whole.
    Cache,
    /// A download's bytes still to come, on its folder's disk.
    Download,
    /// A download's room for its batches, on the data folder's disk
    /// (`download::SCRATCH_BYTES`), held while it runs.
    Scratch,
}

/// Room a program holds on a disk, for one stick. Counts only while the
/// program holds that stick's lock of the same kind (`crate::space`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpaceClaim {
    pub stick_id: StickId,
    pub kind: ClaimKind,
    /// Which disk (`crate::space::Disks::key`).
    pub disk: String,
    pub bytes: u64,
}

/// A stick id replaced by a new link (ADR 0009 §5): what is left of it are
/// its secrets and cache, until the stick that replaced it has its drive
/// marker naming it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetiredStick {
    pub old: StickId,
    pub replaced_by: StickId,
}
