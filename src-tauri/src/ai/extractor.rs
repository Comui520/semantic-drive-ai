use std::fs;
use std::io::{BufReader, Read};
use std::path::Path;

/// Extract text from plain text files
pub fn extract_text_plain(path: &Path) -> Result<String, String> {
    let mut content = String::new();
    let file = fs::File::open(path)
        .map_err(|e| format!("Cannot open {}: {}", path.display(), e))?;
    let mut reader = BufReader::new(file);
    reader
        .read_to_string(&mut content)
        .map_err(|e| format!("Cannot read {}: {}", path.display(), e))?;

    if content.len() > 1_000_000 {
        content.truncate(1_000_000);
    }
    Ok(content)
}

/// Extract text from PDF files using lopdf
pub fn extract_text_pdf(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path)
        .map_err(|e| format!("Cannot read PDF: {}", e))?;

    let doc = lopdf::Document::load_mem(&bytes)
        .map_err(|e| format!("Cannot parse PDF: {}", e))?;

    let mut text = String::new();
    for (page_num, _page_id) in doc.get_pages() {
        if let Ok(content) = doc.extract_text(&[page_num]) {
            text.push_str(&content);
            text.push('\n');
        }
    }

    if text.is_empty() {
        return Err("No extractable text in PDF".to_string());
    }
    if text.len() > 1_000_000 {
        text.truncate(1_000_000);
    }
    Ok(text)
}

/// Extract text from DOCX files (ZIP archive of XML)
pub fn extract_text_docx(path: &Path) -> Result<String, String> {
    let file = fs::File::open(path)
        .map_err(|e| format!("Cannot open DOCX: {}", e))?;
    let reader = BufReader::new(file);

    let mut archive =
        zip::ZipArchive::new(reader).map_err(|e| format!("Cannot read DOCX archive: {}", e))?;

    let doc_xml = archive
        .by_name("word/document.xml")
        .map_err(|e| format!("Cannot find word/document.xml: {}", e))?;

    // Read the XML content into memory first
    let mut xml_content = String::new();
    let mut buf_reader = BufReader::new(doc_xml);
    buf_reader
        .read_to_string(&mut xml_content)
        .map_err(|e| format!("Cannot read DOCX XML: {}", e))?;

    let mut text = String::new();
    let mut reader = quick_xml::Reader::from_str(&xml_content);
    reader.config_mut().trim_text(true);

    let mut buf = Vec::new();
    let mut in_text = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(quick_xml::events::Event::Start(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if name == "w:t" {
                    in_text = true;
                } else if name == "w:p" && !text.is_empty() && !text.ends_with('\n') {
                    text.push('\n');
                }
            }
            Ok(quick_xml::events::Event::End(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if name == "w:t" {
                    in_text = false;
                }
            }
            Ok(quick_xml::events::Event::Text(ref e)) => {
                if in_text {
                    if let Ok(t) = e.unescape() {
                        text.push_str(&t);
                    }
                }
            }
            Ok(quick_xml::events::Event::Eof) => break,
            Err(e) => return Err(format!("XML parse error: {}", e)),
            _ => {}
        }
        buf.clear();
    }

    if text.trim().is_empty() {
        return Err("No extractable text in DOCX".to_string());
    }
    if text.len() > 1_000_000 {
        text.truncate(1_000_000);
    }
    Ok(text.trim().to_string())
}

/// Extract text from XLSX/XLS files using calamine's format auto-detection.
pub fn extract_text_spreadsheet(path: &Path) -> Result<String, String> {
    use calamine::{open_workbook_auto, Reader};

    let mut workbook = open_workbook_auto(path)
        .map_err(|e| format!("Cannot open spreadsheet: {}", e))?;

    let mut text = String::new();
    let sheet_names = workbook.sheet_names().to_vec();

    for sheet_name in &sheet_names {
        if let Ok(range) = workbook.worksheet_range(sheet_name) {
            text.push_str(&format!("[{}]\n", sheet_name));
            for row in range.rows() {
                let row_text: Vec<String> = row.iter().map(|cell| cell.to_string()).collect();
                let line = row_text.join("\t");
                if !line.trim().is_empty() {
                    text.push_str(&line);
                    text.push('\n');
                }
            }
        }
    }

    if text.trim().is_empty() {
        return Err("No extractable text in spreadsheet".to_string());
    }
    if text.len() > 1_000_000 {
        text.truncate(1_000_000);
    }
    Ok(text)
}
