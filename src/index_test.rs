use rmcp_memex::search::{BM25Config, BM25Index};
pub fn search_bm25(query: &str) -> Vec<String> {
    let config = BM25Config::default();
    if let Ok(index) = BM25Index::new(&config) {
        if let Ok(res) = index.search(query, Some("ai-contexts"), 50) {
            return res.into_iter().map(|(id, _, _)| id).collect();
        }
    }
    vec![]
}
