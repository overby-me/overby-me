use std::path::PathBuf;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Options {
    pub(crate) urls: Vec<String>,
    pub(crate) method: Option<String>,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) data: Option<Vec<u8>>,
    pub(crate) data_raw: bool,
    pub(crate) form_fields: Vec<FormField>,
    pub(crate) output: Option<PathBuf>,
    pub(crate) remote_name: bool,
    pub(crate) location: bool,
    pub(crate) max_redirs: usize,
    pub(crate) verbose: bool,
    pub(crate) silent: bool,
    pub(crate) show_error: bool,
    pub(crate) fail: bool,
    pub(crate) include_headers: bool,
    pub(crate) head: bool,
    pub(crate) user_agent: Option<String>,
    pub(crate) referer: Option<String>,
    pub(crate) cookie: Option<String>,
    pub(crate) cookie_jar: Option<PathBuf>,
    pub(crate) user: Option<String>,
    pub(crate) connect_timeout: Option<Duration>,
    pub(crate) max_time: Option<Duration>,
    pub(crate) insecure: bool,
    pub(crate) compressed: bool,
    pub(crate) dump_header: Option<PathBuf>,
    pub(crate) write_out: Option<String>,
    pub(crate) retry: usize,
    pub(crate) range: Option<String>,
    pub(crate) upload_file: Option<PathBuf>,
    pub(crate) http_version: Option<String>,
    pub(crate) no_keepalive: bool,
    pub(crate) cacert: Option<PathBuf>,
    pub(crate) cert: Option<PathBuf>,
    pub(crate) cert_key: Option<PathBuf>,
    pub(crate) resume_from: Option<String>,   // -C / --continue-at
}

#[derive(Clone, Debug)]
pub struct FormField {
    pub(crate) name: String,
    pub(crate) value: String,
    pub(crate) is_file: bool,
    pub(crate) content_type: Option<String>,
    pub(crate) filename: Option<String>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            urls: Vec::new(),
            method: None,
            headers: Vec::new(),
            data: None,
            data_raw: false,
            form_fields: Vec::new(),
            output: None,
            remote_name: false,
            location: false,
            max_redirs: 50,
            verbose: false,
            silent: false,
            show_error: false,
            fail: false,
            include_headers: false,
            head: false,
            user_agent: None,
            referer: None,
            cookie: None,
            cookie_jar: None,
            user: None,
            connect_timeout: None,
            max_time: None,
            insecure: false,
            compressed: false,
            dump_header: None,
            write_out: None,
            retry: 0,
            range: None,
            upload_file: None,
            http_version: None,
            no_keepalive: false,
            cacert: None,
            cert: None,
            cert_key: None,
            resume_from: None,
        }
    }
}
