//! Attaching a local file or a link to the focused issue, each from a
//! prompt. A file is checked before Linear is asked anything: it must
//! exist, be a regular file and be at most [`MAX_FILE_SIZE`]. Its upload
//! shows its progress in the indicator until Linear answers. Nothing shows
//! before Linear stores the attachment, so a failure has nothing to undo.

use std::fs;
use std::path::{Path, PathBuf};

use super::{Effect, Failure, Modal, State, Update, WriteClass};
use crate::linear::FileUpload;
use crate::linear::model::Attachment;

/// The largest file lrat uploads, in bytes.
pub const MAX_FILE_SIZE: u64 = 50_000_000;

/// The open attach prompt: which issue it attaches to, what it asks for
/// now, what has been typed, and why the last try was refused.
#[derive(Debug, Clone, PartialEq)]
pub struct AttachPrompt {
    pub issue_id: String,
    pub identifier: String,
    /// The issue's team, where a refusal teaches what the API key may do.
    pub team_id: String,
    pub step: AttachStep,
    pub text: String,
    pub error: Option<String>,
    /// The entries the last Tab could not choose between.
    pub matches: Vec<String>,
}

/// What the attach prompt asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttachStep {
    /// The path of a file.
    File,
    /// The URL of a link.
    Link,
    /// An optional title for the link to `url`.
    Title { url: String },
}

impl AttachPrompt {
    /// What the prompt's border says, as `Attach a file to ENG-12`.
    pub fn title(&self) -> String {
        match self.step {
            AttachStep::File => format!("Attach a file to {}", self.identifier),
            AttachStep::Link => format!("Attach a link to {}", self.identifier),
            AttachStep::Title { .. } => format!("Title the link on {}", self.identifier),
        }
    }
}

/// Which upload a progress report is about, unique in a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UploadId(u64);

/// An attachment on its way to Linear.
#[derive(Debug, Clone, PartialEq)]
pub struct NewAttachment {
    pub issue_id: String,
    pub identifier: String,
    pub team_id: String,
    pub what: Attaching,
}

/// What [`NewAttachment`] attaches.
#[derive(Debug, Clone, PartialEq)]
pub enum Attaching {
    File {
        upload: UploadId,
        file: FileUpload,
    },
    /// A link, titled `title`, or as Linear names it when `None`.
    Link {
        url: String,
        title: Option<String>,
    },
}

impl NewAttachment {
    /// What messages call it: the file's name or the link's URL.
    pub fn name(&self) -> &str {
        match &self.what {
            Attaching::File { file, .. } => &file.name,
            Attaching::Link { url, .. } => url,
        }
    }

    /// What lrat was doing, as `attach rind.log to ENG-12`.
    pub fn action(&self) -> String {
        format!("attach {} to {}", self.name(), self.identifier)
    }
}

/// One upload in flight: the file's name, its size and how many of its
/// bytes have gone out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upload {
    pub id: UploadId,
    pub name: String,
    pub size: u64,
    pub sent: u64,
}

impl Upload {
    /// How much has gone out, from 0 to 100.
    pub fn percent(&self) -> u64 {
        (self.sent.min(self.size) * 100)
            .checked_div(self.size)
            .unwrap_or(100)
    }
}

/// The uploads in flight, oldest first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Uploads {
    next: u64,
    active: Vec<Upload>,
}

impl Uploads {
    pub fn active(&self) -> &[Upload] {
        &self.active
    }

    fn start(&mut self, name: &str, size: u64) -> UploadId {
        let id = UploadId(self.next);
        self.next += 1;
        self.active.push(Upload {
            id,
            name: name.to_owned(),
            size,
            sent: 0,
        });
        id
    }

    /// Records that `sent` bytes of upload `id` have gone out. Returns
    /// whether its percentage moved, so a redraw shows something new.
    fn progress(&mut self, id: UploadId, sent: u64) -> bool {
        let Some(upload) = self.active.iter_mut().find(|upload| upload.id == id) else {
            return false;
        };
        let before = upload.percent();
        upload.sent = sent;
        upload.percent() != before
    }

    fn finish(&mut self, id: UploadId) {
        self.active.retain(|upload| upload.id != id);
    }
}

/// Opens the prompt for a file to attach to the focused issue.
pub(super) fn open_file(state: &mut State) -> Update {
    open(state, AttachStep::File)
}

/// Opens the prompt for a link to attach to the focused issue.
pub(super) fn open_link(state: &mut State) -> Update {
    open(state, AttachStep::Link)
}

fn open(state: &mut State, step: AttachStep) -> Update {
    let Some(issue) = state.focused_issue() else {
        return Update::default();
    };
    if super::is_local(&issue.id) {
        state.notice = Some(format!(
            "{} is not created yet; attach to it once Linear confirms it",
            issue.identifier
        ));
        return Update::redraw();
    }
    let prompt = AttachPrompt {
        issue_id: issue.id.clone(),
        identifier: issue.identifier.clone(),
        team_id: issue.team_id.clone(),
        step,
        text: String::new(),
        error: None,
        matches: Vec::new(),
    };
    state.modals.push(Modal::Attach(prompt));
    Update::redraw()
}

/// Takes what the open prompt holds: a file that passes the checks starts
/// its upload, a good URL moves on to its title, and a title attaches the
/// link. Anything refused keeps the prompt open with the reason.
pub(super) fn accept(state: &mut State) -> Update {
    let Some(Modal::Attach(prompt)) = state.modals.top_mut() else {
        return Update::default();
    };
    prompt.matches.clear();
    let what = match &prompt.step {
        AttachStep::File => match checked_file(&prompt.text) {
            Ok(file) => {
                let upload = state.uploads.start(&file.name, file.size);
                Attaching::File { upload, file }
            }
            Err(error) => {
                prompt.error = Some(error);
                return Update::redraw();
            }
        },
        AttachStep::Link => {
            let url = prompt.text.trim();
            if !is_web_url(url) {
                prompt.error = Some("Use a URL that starts with http:// or https://".to_owned());
                return Update::redraw();
            }
            prompt.step = AttachStep::Title {
                url: url.to_owned(),
            };
            prompt.text.clear();
            prompt.error = None;
            return Update::redraw();
        }
        AttachStep::Title { url } => {
            let title = prompt.text.trim();
            Attaching::Link {
                url: url.clone(),
                title: (!title.is_empty()).then(|| title.to_owned()),
            }
        }
    };
    let attachment = NewAttachment {
        issue_id: prompt.issue_id.clone(),
        identifier: prompt.identifier.clone(),
        team_id: prompt.team_id.clone(),
        what,
    };
    state.modals.pop();
    Update {
        redraw: true,
        effects: vec![Effect::Attach(attachment)],
    }
}

/// Completes the path in the file prompt from its directory: to the one
/// entry that starts with what was typed, with a `/` after a directory, or
/// as far as several agree, listing them.
pub(super) fn complete(state: &mut State) -> Update {
    let Some(Modal::Attach(prompt)) = state.modals.top_mut() else {
        return Update::default();
    };
    if prompt.step != AttachStep::File {
        return Update::default();
    }
    let (dir, start) = match prompt.text.rfind('/') {
        Some(slash) => prompt.text.split_at(slash + 1),
        None => ("", prompt.text.as_str()),
    };
    let matches = entries(dir, start);
    prompt.matches.clear();
    match matches.as_slice() {
        [] => {
            prompt.error = Some(if start.is_empty() {
                format!("Nothing in {}", if dir.is_empty() { "." } else { dir })
            } else {
                format!("Nothing here starts with {start}")
            });
        }
        [only] => {
            prompt.text = format!("{dir}{only}");
            prompt.error = None;
        }
        several => {
            let shared = shared_start(several);
            prompt.text = format!("{dir}{shared}");
            prompt.error = None;
            prompt.matches = several.to_vec();
        }
    }
    Update::redraw()
}

/// Shows how far upload `id` has gone.
pub(super) fn progressed(state: &mut State, id: UploadId, sent: u64) -> Update {
    Update {
        redraw: state.uploads.progress(id, sent),
        effects: Vec::new(),
    }
}

/// Takes in Linear's answer to `attachment`: the attachment joins the
/// issue's, replacing one with the same URL as Linear does, or the
/// failure goes to the notice and the error log.
pub(super) fn attached(
    state: &mut State,
    attachment: &NewAttachment,
    result: Result<Attachment, Failure>,
) -> Update {
    if let Attaching::File { upload, .. } = &attachment.what {
        state.uploads.finish(*upload);
    }
    let outcome = result.as_ref().map(|_| ()).map_err(|failure| &failure.kind);
    state.learn_write(&attachment.team_id, WriteClass::Attach, outcome);
    match result {
        Ok(stored) => {
            state.notice = Some(format!(
                "Attached {} to {}",
                stored.title, attachment.identifier
            ));
            state.store.add_attachment(stored);
        }
        Err(failure) => {
            state.notice = Some(format!(
                "Could not attach {} to {}: {failure}",
                attachment.name(),
                attachment.identifier
            ));
            state.record(failure);
        }
    }
    Update::redraw()
}

/// The file `typed` names, `~` expanded, if it can be attached, or why
/// not.
fn checked_file(typed: &str) -> Result<FileUpload, String> {
    let typed = typed.trim();
    if typed.is_empty() {
        return Err("Type the path of a file".to_owned());
    }
    let path = expand_home(typed);
    let metadata = match fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(format!("No file at {typed}"));
        }
        Err(error) => return Err(format!("Cannot read {typed}: {error}")),
    };
    if metadata.is_dir() {
        return Err(format!("{typed} is a directory"));
    }
    if !metadata.is_file() {
        return Err(format!("{typed} is not a regular file"));
    }
    let name = path.file_name().map_or_else(
        || typed.to_owned(),
        |name| name.to_string_lossy().into_owned(),
    );
    let size = metadata.len();
    if size > MAX_FILE_SIZE {
        return Err(format!(
            "{name} is {}; the limit is {}",
            megabytes(size),
            megabytes_whole(MAX_FILE_SIZE)
        ));
    }
    Ok(FileUpload {
        content_type: content_type(&path).to_owned(),
        path,
        name,
        size,
    })
}

/// `size` in megabytes with one decimal, as `60.0 MB`.
fn megabytes(size: u64) -> String {
    let tenths = size.div_ceil(100_000);
    format!("{}.{} MB", tenths / 10, tenths % 10)
}

/// `size` in whole megabytes, as `50 MB`.
fn megabytes_whole(size: u64) -> String {
    format!("{} MB", size / 1_000_000)
}

/// `typed` with a leading `~` standing for the home directory.
fn expand_home(typed: &str) -> PathBuf {
    let rest = match typed.strip_prefix('~') {
        Some("") => "",
        Some(rest) if rest.starts_with('/') => rest.trim_start_matches('/'),
        _ => return PathBuf::from(typed),
    };
    match std::env::var_os("HOME") {
        Some(home) => Path::new(&home).join(rest),
        None => PathBuf::from(typed),
    }
}

/// The names in directory `dir`, as typed, that start with `start`, each
/// directory's with a `/` after it, sorted. Hidden entries only when
/// `start` asks for them.
fn entries(dir: &str, start: &str) -> Vec<String> {
    let path = if dir.is_empty() {
        PathBuf::from(".")
    } else {
        expand_home(dir)
    };
    let Ok(read) = fs::read_dir(path) else {
        return Vec::new();
    };
    let mut names: Vec<String> = read
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            if !name.starts_with(start) || (name.starts_with('.') && !start.starts_with('.')) {
                return None;
            }
            // Through the entry's target, so a link to a directory counts.
            let is_dir = fs::metadata(entry.path()).is_ok_and(|metadata| metadata.is_dir());
            Some(if is_dir { format!("{name}/") } else { name })
        })
        .collect();
    names.sort();
    names
}

/// The longest start every one of `names` shares.
fn shared_start(names: &[String]) -> String {
    let Some((first, rest)) = names.split_first() else {
        return String::new();
    };
    let mut shared = first.as_str();
    for name in rest {
        let common = shared
            .char_indices()
            .zip(name.chars())
            .find(|((_, a), b)| a != b)
            .map_or(shared.len().min(name.len()), |((at, _), _)| at);
        shared = &shared[..common];
    }
    shared.to_owned()
}

/// Whether `url` is http or https with a host.
fn is_web_url(url: &str) -> bool {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"));
    rest.and_then(|rest| rest.split(['/', '?', '#']).next())
        .is_some_and(|host| !host.is_empty() && !host.contains(char::is_whitespace))
}

/// The MIME type of `path` by its extension, `application/octet-stream`
/// when it is not one lrat knows.
fn content_type(path: &Path) -> &'static str {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    match extension.as_deref() {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("pdf") => "application/pdf",
        Some("txt" | "log") => "text/plain",
        Some("md") => "text/markdown",
        Some("csv") => "text/csv",
        Some("html" | "htm") => "text/html",
        Some("json") => "application/json",
        Some("toml") => "application/toml",
        Some("yaml" | "yml") => "application/yaml",
        Some("xml") => "application/xml",
        Some("zip") => "application/zip",
        Some("gz") => "application/gzip",
        Some("tar") => "application/x-tar",
        Some("mp4") => "video/mp4",
        Some("mov") => "video/quicktime",
        Some("webm") => "video/webm",
        Some("mp3") => "audio/mpeg",
        Some("wav") => "audio/wav",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_in_megabytes() {
        assert_eq!(megabytes(60_000_000), "60.0 MB");
        assert_eq!(megabytes(50_000_001), "50.1 MB");
        assert_eq!(megabytes_whole(MAX_FILE_SIZE), "50 MB");
    }

    #[test]
    fn shared_start_is_as_far_as_every_name_agrees() {
        let names = ["Cargo.lock", "Cargo.toml"].map(str::to_owned);
        assert_eq!(shared_start(&names), "Cargo.");
        let names = ["cave/", "cave"].map(str::to_owned);
        assert_eq!(shared_start(&names), "cave");
    }

    #[test]
    fn only_web_urls_with_a_host_pass() {
        assert!(is_web_url("https://example.com/cave"));
        assert!(is_web_url("http://example.com"));
        assert!(!is_web_url("https://"));
        assert!(!is_web_url("https:///cave"));
        assert!(!is_web_url("ftp://example.com"));
        assert!(!is_web_url("example.com"));
    }

    #[test]
    fn progress_never_passes_the_whole() {
        let upload = |sent, size| Upload {
            id: UploadId(0),
            name: String::new(),
            size,
            sent,
        };
        assert_eq!(upload(42, 100).percent(), 42);
        assert_eq!(upload(200, 100).percent(), 100);
        assert_eq!(upload(0, 0).percent(), 100);
    }
}
