use crate::MyCadApp;

impl MyCadApp {
    /// Save the parametric document to a JSON file (native: rfd dialog;
    /// WASM: triggers a browser download).
    pub(crate) fn save_param_document(&mut self) -> Result<(), String> {
        let json = serde_json::to_string_pretty(&self.document)
            .map_err(|e| format!("Serialization failed: {}", e))?;

        #[cfg(not(target_arch = "wasm32"))]
        {
            let Some(path) = rfd::FileDialog::new()
                .add_filter("MyCad document", &["json"])
                .set_file_name("mycad_document.json")
                .save_file()
            else {
                self.status_message = "Save cancelled".to_string();
                return Ok(());
            };
            std::fs::write(&path, json)
                .map_err(|e| format!("Failed to write file: {}", e))?;
            self.status_message = format!("Document saved to: {}", path.display());
        }

        #[cfg(target_arch = "wasm32")]
        {
            download_bytes_in_browser(json.as_bytes(), "mycad_document.json", "application/json")?;
            self.status_message = "Document downloaded".to_string();
        }

        Ok(())
    }

    /// Load a parametric document from a JSON file (native only for now;
    /// WASM open uses an async file input handled elsewhere).
    pub(crate) fn load_param_document(&mut self) -> Result<(), String> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let Some(path) = rfd::FileDialog::new()
                .add_filter("MyCad document", &["json"])
                .pick_file()
            else {
                self.status_message = "Open cancelled".to_string();
                return Ok(());
            };
            let contents = std::fs::read_to_string(&path)
                .map_err(|e| format!("Failed to read file: {}", e))?;
            let document = serde_json::from_str(&contents)
                .map_err(|e| format!("Deserialization failed: {}", e))?;
            self.document = document;
            self.sub_editor = None;
            self.rebuild_pending_since = None;
            self.status_message = format!("Document loaded from: {}", path.display());
        }

        #[cfg(target_arch = "wasm32")]
        {
            self.status_message = "Open not yet supported in WASM build".to_string();
        }

        Ok(())
    }

    /// Export the current tessellated solid as binary STL.
    pub(crate) fn export_stl(&mut self) -> Result<(), String> {
        let mesh = self.latest_mesh().ok_or_else(|| {
            "No geometry to export — build a solid first".to_string()
        })?;
        let bytes = mycad_kernel::export::mesh_to_stl_binary(&mesh);

        #[cfg(not(target_arch = "wasm32"))]
        {
            let Some(path) = rfd::FileDialog::new()
                .add_filter("STL", &["stl"])
                .set_file_name("mycad_export.stl")
                .save_file()
            else {
                self.status_message = "STL export cancelled".to_string();
                return Ok(());
            };
            std::fs::write(&path, &bytes)
                .map_err(|e| format!("Failed to write STL: {}", e))?;
            self.status_message = format!("STL exported to: {}", path.display());
        }

        #[cfg(target_arch = "wasm32")]
        {
            download_bytes_in_browser(&bytes, "mycad_export.stl", "application/octet-stream")?;
            self.status_message = "STL downloaded".to_string();
        }

        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
fn download_bytes_in_browser(bytes: &[u8], filename: &str, mime: &str) -> Result<(), String> {
    use wasm_bindgen::JsCast;

    let window = web_sys::window().ok_or_else(|| "no window".to_string())?;
    let document = window.document().ok_or_else(|| "no document".to_string())?;

    let array = js_sys::Uint8Array::new_with_length(bytes.len() as u32);
    array.copy_from(bytes);
    let parts = js_sys::Array::new();
    parts.push(&array.buffer());

    let options = web_sys::BlobPropertyBag::new();
    options.set_type(mime);
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &options)
        .map_err(|e| format!("blob: {:?}", e))?;
    let url = web_sys::Url::create_object_url_with_blob(&blob)
        .map_err(|e| format!("url: {:?}", e))?;

    let anchor: web_sys::HtmlAnchorElement = document
        .create_element("a")
        .map_err(|e| format!("anchor: {:?}", e))?
        .dyn_into()
        .map_err(|_| "not an anchor".to_string())?;
    anchor.set_href(&url);
    anchor.set_download(filename);
    anchor.click();
    let _ = web_sys::Url::revoke_object_url(&url);
    Ok(())
}
