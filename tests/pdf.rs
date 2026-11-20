//! The PDF of a document: it opens, its text can be read back (headings,
//! paragraphs in Ukrainian and Arabic, a table), pending suggestions are
//! not applied, and the people who sign are named at the end.

use serde_json::json;

#[test]
fn a_document_becomes_a_pdf_whose_text_reads_back() {
    let doc = json!({"type": "doc", "content": [
        {"type": "heading", "attrs": {"level": 1}, "content": [{"type": "text", "text": "Flood response plan 2026"}]},
        {"type": "paragraph", "content": [
            {"type": "text", "text": "Each district opens at least one shelter. "},
            {"type": "text", "text": "District 4 uses the sports hall.", "marks": [{"type": "strong"}]},
            {"type": "text", "text": " Not yet accepted.", "marks": [{"type": "insertion", "attrs": {"id": "s1", "author": "Ana"}}]}
        ]},
        {"type": "paragraph", "content": [{"type": "text", "text": "Кожен район відкриває щонайменше один пункт тимчасового розміщення."}]},
        {"type": "paragraph", "content": [{"type": "text", "text": "تفتح كل منطقة ملجأً واحدًا على الأقل."}]},
        {"type": "bullet_list", "content": [{"type": "list_item", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Buses leave from the central depot."}]}]}]},
        {"type": "table", "content": [
            {"type": "table_row", "content": [
                {"type": "table_header", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "District"}]}]},
                {"type": "table_header", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Shelter"}]}]}
            ]},
            {"type": "table_row", "content": [
                {"type": "table_cell", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "4"}]}]},
                {"type": "table_cell", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Sports hall"}]}]}
            ]}
        ]}
    ]});
    let long: Vec<serde_json::Value> = (0..80).map(|i| json!({"type": "paragraph", "content": [{"type": "text", "text": format!("Line {i} of the annex, long enough to wrap across the width of an A4 page with its margins.")}]})).collect();
    let mut doc = doc;
    doc["content"].as_array_mut().unwrap().extend(long);
    let who = [
        ortus_docs::pdf::Signatory {
            name: "Marek Nowak".into(),
            role: "Author signs".into(),
        },
        ortus_docs::pdf::Signatory {
            name: "Katarzyna Lis".into(),
            role: "Director signs".into(),
        },
    ];
    let bytes = ortus_docs::pdf::render(
        "Flood response plan 2026",
        &doc,
        Some("official-sensitive"),
        &who,
        "Signatures",
    );
    assert!(bytes.starts_with(b"%PDF-"));
    if let Ok(path) = std::env::var("ORTUS_TEST_PDF_OUT") {
        std::fs::write(path, &bytes).unwrap();
    }
    assert!(
        bytes.windows(4).any(|w| w == b"xref"),
        "a classic cross-reference table"
    );

    // Read back with Poppler when it is there (CI installs it).
    let Ok(tool) = std::env::var("ORTUS_TEST_PDFTOTEXT").or_else(|_| which("pdftotext")) else {
        assert!(
            std::env::var("ORTUS_REQUIRE_PDFTOTEXT").is_err(),
            "ORTUS_REQUIRE_PDFTOTEXT is set but pdftotext is missing"
        );
        return;
    };
    let dir = std::env::temp_dir().join(format!("ortus-docs-pdf-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("plan.pdf");
    std::fs::write(&file, &bytes).unwrap();
    let out = std::process::Command::new(tool)
        .arg("-layout")
        .arg(&file)
        .arg("-")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    for want in [
        "OFFICIAL-SENSITIVE",
        "Flood response plan 2026",
        "District 4 uses the sports hall.",
        "Кожен район відкриває",
        "Buses leave from the central depot.",
        "District",
        "Sports hall",
        "Line 79 of the annex",
        "Marek Nowak",
        "Director signs",
        "Signatures",
    ] {
        assert!(text.contains(want), "{want:?} is not in:\n{text}");
    }
    assert!(
        !text.contains("Not yet accepted"),
        "a pending suggestion was applied"
    );
    // Arabic comes back as its letters (shaped forms map to them).
    assert!(
        text.contains("ملجأ")
            || text.contains("ﻣﻠﺠﺄ")
            || text.chars().any(|c| ('\u{FE70}'..='\u{FEFF}').contains(&c)
                || ('\u{0600}'..='\u{06FF}').contains(&c)),
        "no Arabic in:\n{text}"
    );
    assert!(
        text.contains("3 / 3") || text.contains("2 / 2") || text.contains(" / "),
        "page numbers"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

fn which(name: &str) -> Result<String, std::env::VarError> {
    std::env::var("PATH")?
        .split(':')
        .map(|d| format!("{d}/{name}"))
        .find(|p| std::path::Path::new(p).exists())
        .ok_or(std::env::VarError::NotPresent)
}
