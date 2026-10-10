//! A stable, extension-based key shared by the map and file tables.
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Media,
    Archives,
    Documents,
    Code,
    Applications,
    Data,
    System,
    Other,
}

impl Category {
    pub const ALL: [Self; 8] = [
        Self::Media,
        Self::Archives,
        Self::Documents,
        Self::Code,
        Self::Applications,
        Self::Data,
        Self::System,
        Self::Other,
    ];

    pub fn from_name(name: &str) -> Self {
        Path::new(name)
            .extension()
            .and_then(|ext| ext.to_str())
            .map(Self::from_extension)
            .unwrap_or(Self::Other)
    }

    pub fn from_extension(extension: &str) -> Self {
        match extension
            .trim_start_matches('.')
            .to_ascii_lowercase()
            .as_str()
        {
            "mp4" | "mkv" | "mov" | "avi" | "webm" | "wmv" | "m4v" | "mpeg" | "mpg" | "mts"
            | "m2ts" | "flv" => Self::Media,
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "avif" | "bmp" | "tif" | "tiff" | "svg"
            | "ico" | "heic" | "psd" | "tga" | "exr" | "hdr" | "dds" | "ktx" | "ktx2" => {
                Self::Media
            }
            "mp3" | "wav" | "flac" | "ogg" | "opus" | "aac" | "m4a" | "wma" | "aiff" | "aif"
            | "mid" | "midi" => Self::Media,
            "zip" | "7z" | "rar" | "tar" | "gz" | "bz2" | "xz" | "zst" | "cab" | "iso" | "vhd"
            | "vhdx" => Self::Archives,
            "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "odt" | "ods" | "odp"
            | "rtf" | "txt" | "md" | "csv" | "epub" | "log" => Self::Documents,
            "rs" | "py" | "js" | "jsx" | "ts" | "tsx" | "css" | "html" | "htm" | "json"
            | "toml" | "yaml" | "yml" | "xml" | "ini" | "cfg" | "c" | "h" | "cpp" | "hpp"
            | "cs" | "go" | "java" | "sh" | "ps1" | "bat" | "cmd" | "sql" | "vue" | "svelte"
            | "ipynb" => Self::Code,
            "exe" | "dll" | "msi" | "msix" | "appx" | "com" | "so" | "dylib" | "lib" | "pdb" => {
                Self::Applications
            }
            "bin" | "dat" | "db" | "sqlite" | "sqlite3" | "mdb" | "accdb" | "vpk" | "pak"
            | "bundle" | "assets" | "asset" | "unity3d" | "uasset" | "umap" | "blend" | "fbx"
            | "obj" | "glb" | "gltf" | "cache" | "safetensors" | "onnx" | "pt" | "pth" | "ckpt"
            | "npy" | "npz" | "parquet" => Self::Data,
            "sys" | "drv" | "mui" | "cat" | "inf" | "etl" | "evtx" => Self::System,
            _ => Self::Other,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Media => "Media",
            Self::Archives => "Archives",
            Self::Documents => "Documents",
            Self::Code => "Code",
            Self::Applications => "Applications",
            Self::Data => "Data & assets",
            Self::System => "System",
            Self::Other => "Other",
        }
    }

    pub fn examples(self) -> &'static str {
        match self {
            Self::Media => "Video, images and audio: MP4, PNG, JPEG, MP3, WAV",
            Self::Archives => "ZIP, 7Z, RAR, disk images",
            Self::Documents => "PDF, Office, text, logs",
            Self::Code => "Rust, Python, JavaScript, configuration",
            Self::Applications => "Executables, libraries, installers",
            Self::Data => "Databases, game bundles, 3D assets",
            Self::System => "System and driver file extensions",
            Self::Other => "Unrecognised or missing extensions",
        }
    }

    /// Muted category hues: blue, gold, green, yellow, violet, orange and grey.
    /// Grey is reserved for Other, so folders and grouped files never read as a file type.
    pub fn rgb(self) -> [u8; 3] {
        match self {
            Self::Media => [66, 177, 204],
            Self::Archives => [220, 143, 60],
            Self::Documents => [55, 170, 133],
            Self::Code => [215, 200, 102],
            Self::Applications => [52, 110, 184],
            Self::Data => [163, 129, 207],
            Self::System => [224, 126, 106],
            Self::Other => [137, 151, 164],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories_are_stable_across_case_and_extension_labels() {
        for (name, extension, category) in [
            ("clip.MP4", ".mp4", Category::Media),
            ("texture.PNG", ".PNG", Category::Media),
            ("archive.7z", "7z", Category::Archives),
            ("main.rs", ".rs", Category::Code),
            ("config.JSON", "json", Category::Code),
            ("engine.DLL", ".dll", Category::Applications),
            ("shared.bundle", ".bundle", Category::Data),
            ("weights.SAFETENSORS", "safetensors", Category::Data),
            ("pagefile.sys", "sys", Category::System),
        ] {
            assert_eq!(Category::from_name(name), category);
            assert_eq!(Category::from_extension(extension), category);
        }
    }

    #[test]
    fn unknown_files_stay_other_and_generic_data_does_not_imply_an_application() {
        assert_eq!(Category::from_name("README"), Category::Other);
        assert_eq!(Category::from_name(".gitignore"), Category::Other);
        assert_eq!(Category::from_extension("No extension"), Category::Other);
        assert_eq!(Category::from_name("file.unknown"), Category::Other);
        assert_eq!(Category::from_name("payload.bin"), Category::Data);
        assert_eq!(Category::from_name("database.dat"), Category::Data);
        assert_eq!(Category::from_name("index.ts"), Category::Code);
    }
}
