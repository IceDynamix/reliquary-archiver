pub mod file_download;
pub mod file_picker;

#[derive(Debug, Clone)]
pub struct FileExtensions {
    pub description: String,
    pub extensions: Vec<String>,
}

impl FileExtensions {
    pub fn of(description: &str, extensions: &[&str]) -> Self {
        Self { description: description.to_string(), extensions: extensions.iter().map(|e| e.to_string()).collect() }
    }
}

#[derive(Debug, Clone)]
pub struct FileContainer {
    pub name: String,
    pub content: String,
    pub ext: FileExtensions,
}

