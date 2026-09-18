use chrono::Datelike;
use serde::{Deserialize, Serialize};
use crate::ai::search::split_terms;

/// Structured result from natural-language query parsing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedQuery {
    pub keywords: Vec<String>,
    pub time_range: Option<(String, String)>,
    pub file_types: Vec<String>,
    pub entities: Vec<String>,
    pub original: String,
}

/// Cloud generation options shared with the OpenAI-compatible chat client.
#[derive(Debug, Clone)]
pub struct GenerateConfig {
    pub max_tokens: usize,
    pub temperature: f64,
}

impl Default for GenerateConfig {
    fn default() -> Self { Self { max_tokens: 512, temperature: 0.4 } }
}

/// Rule-based query parser used locally. Chat generation is always delegated to
/// the configured cloud API, so no model weights are bundled with the desktop app.
pub struct LlmEngine;

impl Default for LlmEngine {
    fn default() -> Self { Self::new() }
}

impl LlmEngine {
    pub fn new() -> Self { Self }

    pub fn parse_query(&self, query: &str) -> ParsedQuery {
        self.fallback_parse(query)
    }
    // ── Rule-based fallback parsing ──

    fn fallback_parse(&self, query: &str) -> ParsedQuery {
        let query_trimmed = query.trim();

        // Extract file type hints
        let file_types = self.extract_file_types(query_trimmed);

        // Extract time range (removes matched phrases from query)
        let (time_range, remaining_after_time) = self.extract_time_range(query_trimmed);

        // Build keywords from remaining text
        let keywords = self.extract_keywords(&remaining_after_time, &file_types);

        // Simple entity extraction: 2+ char Chinese words
        let entities = self.extract_entities(query_trimmed);

        ParsedQuery {
            keywords,
            time_range,
            file_types,
            entities,
            original: query_trimmed.to_string(),
        }
    }

    fn extract_file_types(&self, query: &str) -> Vec<String> {
        // Chinese category → file extension mapping
        const CHINESE_CATEGORIES: &[(&[&str], &[&str])] = &[
            (&["图片", "照片", "图像", "截图", "壁纸", "摄影"], &["jpg", "jpeg", "png", "gif", "webp", "bmp", "svg", "ico"]),
            (&["文档", "文字", "文稿", "文章"], &["pdf", "doc", "docx", "txt", "md", "rtf"]),
            (&["视频", "电影", "影片", "录制", "录像"], &["mp4", "avi", "mkv", "mov", "wmv", "flv", "webm"]),
            (&["音乐", "音频", "歌曲", "录音", "语音"], &["mp3", "wav", "flac", "aac", "ogg", "wma"]),
            (&["表格", "报表", "统计", "电子表格", "excel"], &["xls", "xlsx", "csv", "tsv"]),
            (&["代码", "程序", "脚本", "编程", "源码"], &["rs", "py", "js", "ts", "java", "go", "c", "cpp", "rb", "php", "swift"]),
            (&["演示", "幻灯片", "演示文稿", "演讲"], &["ppt", "pptx", "key"]),
            (&["压缩", "压缩包", "归档", "备份"], &["zip", "rar", "7z", "tar", "gz", "bz2"]),
            (&["电子书", "书籍", "小说", "图书", "读物"], &["epub", "mobi", "azw3", "pdf"]),
            (&["设计", "设计稿", "UI", "平面", "3D"], &["psd", "ai", "svg", "fig", "sketch"]),
        ];

        // Also handle explicit extension mentions (.pdf, .docx) and bare names (pdf, docx)
        let explicit_re = regex::Regex::new(r"\.(\w{2,4})").unwrap();
        let bare_extensions = ["pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx",
            "txt", "png", "jpg", "jpeg", "gif", "zip", "rar", "mp3", "mp4", "csv",
            "md", "html", "json", "xml", "py", "rs", "js", "ts", "go", "java"];
        let query_lower = query.to_lowercase();
        let mut types: Vec<String> = Vec::new();

        // Check Chinese categories first
        for (category_keywords, extensions) in CHINESE_CATEGORIES {
            if category_keywords.iter().any(|kw| query_lower.contains(kw)) {
                for ext in *extensions {
                    if !types.contains(&ext.to_string()) {
                        types.push(ext.to_string());
                    }
                }
            }
        }

        // Also extract explicit extensions like .pdf, .docx
        for cap in explicit_re.captures_iter(&query_lower) {
            let ext = cap[1].to_string();
            if !types.contains(&ext) {
                types.push(ext);
            }
        }

        // Detect bare extension names like "pdf", "docx" without dot prefix.
        // Use whole-word matching to avoid false positives like "python" matching "py".
        let terms = split_terms(&query_lower);
        for ext in &bare_extensions {
            if terms.iter().any(|t| t == ext) && !types.contains(&ext.to_string()) {
                types.push(ext.to_string());
            }
        }

        types
    }

    /// Extract time range from query. Returns (range, remaining_query).
    /// The remaining query has matched time phrases removed for keyword extraction.
    fn extract_time_range(&self, query: &str) -> (Option<(String, String)>, String) {
        use chrono::Local;
        let today = Local::now().naive_local().date();

        // Check each pattern in priority order (most specific first)
        // If a pattern matches, we remove that phrase from the query for keyword extraction

        // 1. Absolute date: YYYY年M月D日
        let re = regex::Regex::new(r"(\d{4})年(\d{1,2})月(\d{1,2})日").unwrap();
        if let Some(caps) = re.captures(query) {
            let y = caps[1].parse::<i32>().unwrap_or(2026);
            let m = caps[2].parse::<u32>().unwrap_or(1);
            let d = caps[3].parse::<u32>().unwrap_or(1);
            if let Some(date) = chrono::NaiveDate::from_ymd_opt(y, m, d) {
                let ds = date.format("%Y-%m-%d").to_string();
                let remaining = re.replace(query, "").to_string();
                return (Some((ds.clone(), ds)), remaining);
            }
        }

        // 2. Year before: YYYY年以前 / YYYY年之前
        let re = regex::Regex::new(r"(\d{4})年(?:以前|之前)").unwrap();
        if let Some(caps) = re.captures(query) {
            let y = caps[1].parse::<i32>().unwrap_or(2026);
            let end = chrono::NaiveDate::from_ymd_opt(y - 1, 12, 31)
                .unwrap().format("%Y-%m-%d").to_string();
            let remaining = re.replace(query, "").to_string();
            return (Some(("".to_string(), end)), remaining);
        }

        // 3. Year after: YYYY年以后 / YYYY年之后
        let re = regex::Regex::new(r"(\d{4})年(?:以后|之后)").unwrap();
        if let Some(caps) = re.captures(query) {
            let y = caps[1].parse::<i32>().unwrap_or(2026);
            let start = chrono::NaiveDate::from_ymd_opt(y, 1, 1)
                .unwrap().format("%Y-%m-%d").to_string();
            let remaining = re.replace(query, "").to_string();
            return (Some((start, "".to_string())), remaining);
        }

        // 4. Year range: YYYY到YYYY
        let re = regex::Regex::new(r"(\d{4})到(\d{4})").unwrap();
        if let Some(caps) = re.captures(query) {
            let y1 = caps[1].parse::<i32>().unwrap_or(2026);
            let y2 = caps[2].parse::<i32>().unwrap_or(2026);
            let start = chrono::NaiveDate::from_ymd_opt(y1, 1, 1)
                .unwrap().format("%Y-%m-%d").to_string();
            let end = chrono::NaiveDate::from_ymd_opt(y2, 12, 31)
                .unwrap().format("%Y-%m-%d").to_string();
            let remaining = re.replace(query, "").to_string();
            return (Some((start, end)), remaining);
        }

        // 5. Year month with 以前/以后/之前/之后: must come before bare YYYY年M月
        // 5a. YYYY年M月以前/之前
        let re = regex::Regex::new(r"(\d{4})年(\d{1,2})月(?:以前|之前)").unwrap();
        if let Some(caps) = re.captures(query) {
            let y = caps[1].parse::<i32>().unwrap_or(2026);
            let m = caps[2].parse::<u32>().unwrap_or(1);
            let end = if m == 1 {
                chrono::NaiveDate::from_ymd_opt(y - 1, 12, 31).unwrap()
            } else {
                chrono::NaiveDate::from_ymd_opt(y, m, 1).unwrap() - chrono::Duration::days(1)
            };
            let remaining = re.replace(query, "").to_string();
            return (Some(("".to_string(), end.format("%Y-%m-%d").to_string())), remaining);
        }

        // 5b. YYYY年M月以后/之后
        let re = regex::Regex::new(r"(\d{4})年(\d{1,2})月(?:以后|之后)").unwrap();
        if let Some(caps) = re.captures(query) {
            let y = caps[1].parse::<i32>().unwrap_or(2026);
            let m = caps[2].parse::<u32>().unwrap_or(1);
            let start = if m == 12 {
                chrono::NaiveDate::from_ymd_opt(y + 1, 1, 1).unwrap()
            } else {
                chrono::NaiveDate::from_ymd_opt(y, m + 1, 1).unwrap()
            };
            let remaining = re.replace(query, "").to_string();
            return (Some((start.format("%Y-%m-%d").to_string(), "".to_string())), remaining);
        }

        // 5c. M月以前/之前 (current year, only if we're past that month)
        let re = regex::Regex::new(r"(\d{1,2})月(?:以前|之前)").unwrap();
        if let Some(caps) = re.captures(query) {
            let m = caps[1].parse::<u32>().unwrap_or(1);
            let y = today.year();
            if m > 1 && today.month() > m {
                let end = chrono::NaiveDate::from_ymd_opt(y, m, 1).unwrap() - chrono::Duration::days(1);
                let remaining = re.replace(query, "").to_string();
                return (Some(("".to_string(), end.format("%Y-%m-%d").to_string())), remaining);
            } else if m == 1 && today.year() > y {
                let end = chrono::NaiveDate::from_ymd_opt(y - 1, 12, 31).unwrap();
                let remaining = re.replace(query, "").to_string();
                return (Some(("".to_string(), end.format("%Y-%m-%d").to_string())), remaining);
            }
            // Fall through if the time hasn't passed yet
        }

        // 5d. M月以后/之后 (current year)
        let re = regex::Regex::new(r"(\d{1,2})月(?:以后|之后)").unwrap();
        if let Some(caps) = re.captures(query) {
            let m = caps[1].parse::<u32>().unwrap_or(1);
            let y = today.year();
            if m < 12 {
                let start = chrono::NaiveDate::from_ymd_opt(y, m + 1, 1).unwrap();
                let remaining = re.replace(query, "").to_string();
                return (Some((start.format("%Y-%m-%d").to_string(), "".to_string())), remaining);
            } else if m == 12 {
                let start = chrono::NaiveDate::from_ymd_opt(y + 1, 1, 1).unwrap();
                let remaining = re.replace(query, "").to_string();
                return (Some((start.format("%Y-%m-%d").to_string(), "".to_string())), remaining);
            }
        }

        // 5e. Bare 以前/之前 (everything before today)
        let re = regex::Regex::new(r"(?:以前|之前)").unwrap();
        if re.is_match(query) {
            let yesterday = (today - chrono::Duration::days(1)).format("%Y-%m-%d").to_string();
            let remaining = re.replace(query, "").to_string();
            return (Some(("".to_string(), yesterday)), remaining);
        }

        // 5. Year month: YYYY年M月 (must come before bare year)
        let re = regex::Regex::new(r"(\d{4})年(\d{1,2})月").unwrap();
        if let Some(caps) = re.captures(query) {
            let y = caps[1].parse::<i32>().unwrap_or(2026);
            let m = caps[2].parse::<u32>().unwrap_or(1);
            let start = chrono::NaiveDate::from_ymd_opt(y, m, 1).unwrap();
            let end = if m == 12 {
                chrono::NaiveDate::from_ymd_opt(y, 12, 31).unwrap()
            } else {
                chrono::NaiveDate::from_ymd_opt(y, m + 1, 1).unwrap()
                    - chrono::Duration::days(1)
            };
            let remaining = re.replace(query, "").to_string();
            return (Some((
                start.format("%Y-%m-%d").to_string(),
                end.format("%Y-%m-%d").to_string(),
            )), remaining);
        }

        // 6. Bare year: YYYY年
        let re = regex::Regex::new(r"(\d{4})年").unwrap();
        if let Some(caps) = re.captures(query) {
            let y = caps[1].parse::<i32>().unwrap_or(2026);
            let start = chrono::NaiveDate::from_ymd_opt(y, 1, 1)
                .unwrap().format("%Y-%m-%d").to_string();
            let end = chrono::NaiveDate::from_ymd_opt(y, 12, 31)
                .unwrap().format("%Y-%m-%d").to_string();
            let remaining = re.replace(query, "").to_string();
            return (Some((start, end)), remaining);
        }

        // 7. 前年
        if query.contains("前年") {
            let y = today.year() - 2;
            let start = chrono::NaiveDate::from_ymd_opt(y, 1, 1)
                .unwrap().format("%Y-%m-%d").to_string();
            let end = chrono::NaiveDate::from_ymd_opt(y, 12, 31)
                .unwrap().format("%Y-%m-%d").to_string();
            let remaining = query.replace("前年", "");
            return (Some((start, end)), remaining);
        }

        // 8. 去年
        if query.contains("去年") {
            let y = today.year() - 1;
            let start = chrono::NaiveDate::from_ymd_opt(y, 1, 1)
                .unwrap().format("%Y-%m-%d").to_string();
            let end = chrono::NaiveDate::from_ymd_opt(y, 12, 31)
                .unwrap().format("%Y-%m-%d").to_string();
            let remaining = query.replace("去年", "");
            return (Some((start, end)), remaining);
        }

        // 9. 今年
        if query.contains("今年") {
            let y = today.year();
            let start = chrono::NaiveDate::from_ymd_opt(y, 1, 1)
                .unwrap().format("%Y-%m-%d").to_string();
            let remaining = query.replace("今年", "");
            return (Some((start, today.format("%Y-%m-%d").to_string())), remaining);
        }

        // 10. 上上周
        if query.contains("上上周") || query.contains("上上週") {
            let start = (today - chrono::Duration::days(14)).format("%Y-%m-%d").to_string();
            let end = (today - chrono::Duration::days(7)).format("%Y-%m-%d").to_string();
            let remaining = query.replace("上上周", "").replace("上上週", "");
            return (Some((start, end)), remaining);
        }

        // 11. 上周N / 上週N
        let re = regex::Regex::new(r"上[周週]([一二三四五六日])").unwrap();
        if let Some(caps) = re.captures(query) {
            let weekday_map = [("一", 1), ("二", 2), ("三", 3), ("四", 4), ("五", 5), ("六", 6), ("日", 7)];
            let target = &caps[1];
            for (name, num) in &weekday_map {
                if target.contains(name) {
                    let current_weekday = today.format("%u").to_string().parse::<i64>().unwrap_or(1);
                    let last_same_day = today - chrono::Duration::days(current_weekday + 7 - num);
                    let ds = last_same_day.format("%Y-%m-%d").to_string();
                    let remaining = re.replace(query, "").to_string();
                    return (Some((ds.clone(), ds)), remaining);
                }
            }
        }

        // 12. 本周 / 这周
        if query.contains("本周") || query.contains("这周") {
            let weekday = today.format("%u").to_string().parse::<i64>().unwrap_or(1);
            let monday = today - chrono::Duration::days(weekday - 1);
            let remaining = query.replace("本周", "").replace("这周", "");
            return (Some((
                monday.format("%Y-%m-%d").to_string(),
                today.format("%Y-%m-%d").to_string(),
            )), remaining);
        }

        // 13. 上周
        if query.contains("上周") || query.contains("上週") {
            let weekday = today.format("%u").to_string().parse::<i64>().unwrap_or(1);
            let last_monday = today - chrono::Duration::days(weekday + 6);
            let last_sunday = today - chrono::Duration::days(weekday);
            let remaining = query.replace("上周", "").replace("上週", "");
            return (Some((
                last_monday.format("%Y-%m-%d").to_string(),
                last_sunday.format("%Y-%m-%d").to_string(),
            )), remaining);
        }

        // 14. 前天
        if query.contains("前天") {
            let d = (today - chrono::Duration::days(2)).format("%Y-%m-%d").to_string();
            let remaining = query.replace("前天", "");
            return (Some((d.clone(), d)), remaining);
        }

        // 15. 今天
        if query.contains("今天") {
            let d = today.format("%Y-%m-%d").to_string();
            let remaining = query.replace("今天", "");
            return (Some((d.clone(), d)), remaining);
        }

        // 16. 昨天
        if query.contains("昨天") {
            let d = (today - chrono::Duration::days(1)).format("%Y-%m-%d").to_string();
            let remaining = query.replace("昨天", "");
            return (Some((d.clone(), d)), remaining);
        }

        // 17. 本月 / 这个月
        if query.contains("本月") || query.contains("这个月") {
            let start = today.format("%Y-%m-01").to_string();
            let remaining = query.replace("本月", "").replace("这个月", "");
            return (Some((start, today.format("%Y-%m-%d").to_string())), remaining);
        }

        // 18. 上个月 / 上月
        if query.contains("上个月") || query.contains("上月") {
            let first_this = today.format("%Y-%m-01").to_string();
            if let Ok(first_this) = chrono::NaiveDate::parse_from_str(&first_this, "%Y-%m-%d") {
                let last_month_end = first_this - chrono::Duration::days(1);
                let last_month_start = last_month_end.format("%Y-%m-01").to_string();
                let remaining = query.replace("上个月", "").replace("上月", "");
                return (Some((
                    last_month_start,
                    last_month_end.format("%Y-%m-%d").to_string(),
                )), remaining);
            }
        }

        // 19. 最近N个月 / 近N个月
        let re = regex::Regex::new(r"(?:最近|近)(\d+)(?:个)?月").unwrap();
        if let Some(caps) = re.captures(query) {
            if let Ok(n) = caps[1].parse::<i64>() {
                let start = (today - chrono::Duration::days(n * 30)).format("%Y-%m-%d").to_string();
                let remaining = re.replace(query, "").to_string();
                return (Some((start, today.format("%Y-%m-%d").to_string())), remaining);
            }
        }

        // 20. 最近N周 / 近N周
        let re = regex::Regex::new(r"(?:最近|近)(\d+)周").unwrap();
        if let Some(caps) = re.captures(query) {
            if let Ok(n) = caps[1].parse::<i64>() {
                let start = (today - chrono::Duration::days(n * 7)).format("%Y-%m-%d").to_string();
                let remaining = re.replace(query, "").to_string();
                return (Some((start, today.format("%Y-%m-%d").to_string())), remaining);
            }
        }

        // 21. 最近N天 / 近N天
        let re = regex::Regex::new(r"(?:最近|近)(\d+)[天日]").unwrap();
        if let Some(caps) = re.captures(query) {
            if let Ok(n) = caps[1].parse::<i64>() {
                let start = (today - chrono::Duration::days(n)).format("%Y-%m-%d").to_string();
                let remaining = re.replace(query, "").to_string();
                return (Some((start, today.format("%Y-%m-%d").to_string())), remaining);
            }
        }

        (None, query.to_string())
    }

    fn extract_keywords(&self, query: &str, _file_types: &[String]) -> Vec<String> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return Vec::new();
        }

        // Remove time-related and file-type phrases from the text
        let removal_words = [
            // Time
            "今天", "昨天", "前天", "本周", "这周", "上周", "下周", "上上周",
            "本月", "这个月", "上月", "上个月", "下月", "今年", "去年", "前年",
            "最近", "近", "以前", "之前", "以后", "之后",
            // File type
            "文件", "文档", "表格", "图片", "照片", "视频", "音乐", "压缩",
            "目录", "内容", "资料", "信息", "数据", "文件夹",
            // Common query fillers
            "一份", "一个", "一篇", "一本", "一张", "一条",
            "未完成", "已完成", "进行中",
        ];
        let mut cleaned = trimmed.to_string();
        for w in &removal_words {
            cleaned = cleaned.replace(w, "");
        }

        // Remove stop characters (Chinese particles, punctuation, separators)
        // Only include characters that are always grammatical particles and never part of meaningful compounds.
        let stop_chars: Vec<char> = "的了和与或在是有没有很被把将从对到以为就都要让给向往用做查找搜看写改删 ，。？！、；：.!?,()[]{}'\"吗吧呢啊哦啦嘛这那".chars().collect();
        cleaned.retain(|c| !stop_chars.contains(&c));

        // Keep only alphanumeric, underscore, hyphen (drop any leftover special chars)
        let clean: String = cleaned.chars()
            .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
            .collect();
        cleaned = clean;

        let mut keywords: Vec<String> = Vec::new();

        // Extract 3+ digit numbers (years, dates — very important for search)
        let num_re = regex::Regex::new(r"\d{3,}").unwrap();
        for m in num_re.find_iter(&cleaned) {
            let s = m.as_str().to_string();
            if !keywords.contains(&s) {
                keywords.push(s);
            }
        }
        cleaned = num_re.replace_all(&cleaned, "").to_string();

        // Remaining text (2+ characters) as a keyword
        let cleaned = cleaned.trim();
        if cleaned.chars().count() >= 2 {
            keywords.push(cleaned.to_string());
        }

        // Also add split terms from the original query for better Chinese matching
        // Each term goes through the same cleaning pipeline as the main query
        let split_terms = split_terms(trimmed);
        for term in &split_terms {
            let mut ct = term.clone();
            for w in &removal_words {
                ct = ct.replace(w, "");
            }
            ct.retain(|c| !stop_chars.contains(&c));
            let ct: String = ct.chars()
                .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                .collect();
            if ct.chars().count() >= 2 && !keywords.contains(&ct) {
                keywords.push(ct);
            }
        }

        keywords
    }

    fn extract_entities(&self, query: &str) -> Vec<String> {
        let mut entities = Vec::new();
        for c in query.chars() {
            if c.is_whitespace() {
                continue;
            }
        }

        // Find consecutive Chinese characters (2+ chars) as potential entities
        let mut current = String::new();
        for ch in query.chars() {
            if ch >= '\u{4e00}' && ch <= '\u{9fff}' {
                current.push(ch);
            } else {
                if current.len() >= 2 {
                    // Filter out common stop words
                    let stops = ["文件", "文档", "表格", "图片", "照片", "视频", "最近",
                        "今天", "昨天", "本周", "上周", "本月", "上月", "搜索", "查找", "找到"];
                    if !stops.contains(&current.as_str()) {
                        entities.push(current.clone());
                    }
                }
                current.clear();
            }
        }
        if current.len() >= 2 {
            entities.push(current);
        }

        entities
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_file_type_and_relative_date_without_a_model() {
        let query = LlmEngine::new().parse_query("找上周的合同 PDF");
        assert!(query.file_types.iter().any(|kind| kind == "pdf"));
        assert!(query.time_range.is_some());
    }

    #[test]
    fn preserves_original_query() {
        let query = LlmEngine::new().parse_query("搜索项目预算");
        assert_eq!(query.original, "搜索项目预算");
    }
}
