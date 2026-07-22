pub trait QueryRewriteService: Send + Sync {
    fn rewrite(&self, query: &str) -> RewriteResult;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewriteResult {
    pub original_query: String,
    pub normalized_query: String,
    pub sub_queries: Vec<String>,
    pub path_hint: Option<String>,
}

#[derive(Debug, Default)]
pub struct DeterministicQueryRewriteService;

impl QueryRewriteService for DeterministicQueryRewriteService {
    fn rewrite(&self, query: &str) -> RewriteResult {
        let normalized_query = normalize_query(query);
        let sub_queries = split_sub_queries(&normalized_query);
        let path_hint = detect_path_hint(&normalized_query);
        RewriteResult {
            original_query: query.to_string(),
            normalized_query,
            sub_queries,
            path_hint,
        }
    }
}

fn normalize_query(query: &str) -> String {
    let mut normalized = String::with_capacity(query.len());
    let mut last_was_space = true;
    for ch in query.chars() {
        let ch = if ch == '\\' { '/' } else { ch };
        if ch.is_whitespace() {
            if !last_was_space {
                normalized.push(' ');
                last_was_space = true;
            }
        } else {
            normalized.push(ch);
            last_was_space = false;
        }
    }
    normalized.trim().to_string()
}

fn split_sub_queries(query: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;

    for ch in query.chars() {
        if ch == '"' {
            in_quote = !in_quote;
            current.push(ch);
            continue;
        }

        if !in_quote && matches!(ch, '\n' | ';' | '；' | '?' | '？') {
            push_query_part(&mut parts, &mut current);
            if parts.len() >= 4 {
                break;
            }
            continue;
        }

        current.push(ch);
    }

    if parts.len() < 4 {
        push_query_part(&mut parts, &mut current);
    }

    if parts.is_empty() && !query.is_empty() {
        parts.push(query.to_string());
    }

    parts.truncate(4);
    parts
}

fn push_query_part(parts: &mut Vec<String>, current: &mut String) {
    let part = current.trim();
    if !part.is_empty() {
        parts.push(part.to_string());
    }
    current.clear();
}

fn detect_path_hint(query: &str) -> Option<String> {
    query.split_whitespace().find_map(|token| {
        let token = token.trim_matches(|ch: char| {
            matches!(
                ch,
                '"' | '\'' | '`' | ',' | ':' | '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>'
            )
        });
        if !token.contains('/') {
            return None;
        }
        let file_name = token.rsplit('/').next().unwrap_or(token);
        let dot = file_name.rfind('.')?;
        if dot == 0 || dot + 1 >= file_name.len() {
            return None;
        }
        Some(token.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_rewrite_normalizes_whitespace_and_paths() {
        let service = DeterministicQueryRewriteService;

        let result = service.rewrite("  src\\tools\\rag.rs   retrieve   docs  ");

        assert_eq!(result.normalized_query, "src/tools/rag.rs retrieve docs");
        assert_eq!(result.sub_queries, vec!["src/tools/rag.rs retrieve docs"]);
        assert_eq!(result.path_hint.as_deref(), Some("src/tools/rag.rs"));
    }

    #[test]
    fn deterministic_rewrite_splits_multi_question_queries() {
        let service = DeterministicQueryRewriteService;

        let result =
            service.rewrite("How index? How retrieve；manifest fallback？\nscore normalization");

        assert_eq!(
            result.sub_queries,
            vec![
                "How index",
                "How retrieve",
                "manifest fallback",
                "score normalization"
            ]
        );
    }

    #[test]
    fn deterministic_rewrite_preserves_quoted_strings_and_caps_subqueries() {
        let service = DeterministicQueryRewriteService;

        let result = service.rewrite("\"retrieve_docs\"? alpha? beta? gamma? delta? epsilon?");

        assert_eq!(result.sub_queries.len(), 4);
        assert_eq!(result.sub_queries[0], "\"retrieve_docs\"");
    }
}
