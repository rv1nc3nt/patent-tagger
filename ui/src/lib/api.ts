import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface ImportReport {
  imported: string[];
  duplicates: string[];
  needs_normalisation: string[];
  unparseable: string[];
}

export type FetchStatus = "fetched" | "no_english_abstract" | "not_found" | "error";

export interface DocumentOutcome {
  doc_id: number;
  pub_key: string;
  status: FetchStatus;
  title: string | null;
  error: string | null;
  related_pub_keys: string[];
}

export function importNumbers(rawInput: string): Promise<ImportReport> {
  return invoke("import_numbers", { rawInput });
}

export function runImportJobs(): Promise<DocumentOutcome[]> {
  return invoke("run_import_jobs");
}

export function retryDocument(docId: number): Promise<void> {
  return invoke("retry_document", { docId });
}

export function saveOpsCredentials(consumerKey: string, consumerSecret: string): Promise<void> {
  return invoke("save_ops_credentials", { consumerKey, consumerSecret });
}

export function testOpsConnection(): Promise<string> {
  return invoke("test_ops_connection");
}

export function onImportProgress(callback: (outcome: DocumentOutcome) => void): Promise<UnlistenFn> {
  return listen<DocumentOutcome>("import-progress", (event) => callback(event.payload));
}

export interface TagRow {
  id: number;
  name: string;
  definition: string;
  color: string | null;
  hotkey: string | null;
  version: number;
  archived: boolean;
  threshold: number | null;
  auto_enabled: boolean;
}

export interface QueueEntry {
  id: number;
  pub_key: string;
  title: string | null;
}

export interface DocumentDetail {
  id: number;
  pub_key: string;
  title: string | null;
  abstract_text: string | null;
  applicants: string[];
  publication_date: string | null;
  cpc: string[];
  ipc: string[];
  kind_codes: string[];
  application_number: string | null;
  family_id: string | null;
}

export interface TagScore {
  tag_id: number;
  name: string;
  color: string | null;
  hotkey: string | null;
  score: number | null;
  source: "blend" | "lr" | "knn" | "zero_shot" | null;
  model_version: string;
  suggested: boolean;
}

export type FulltextStatus = "pending" | "fetched" | "not_available" | "non_english_only" | "error";

export interface FulltextRow {
  doc_id: number;
  description: string | null;
  claims: string | null;
  lang: string | null;
  source: string | null;
  status: FulltextStatus;
  fetched_at: string | null;
}

export type DrawingsStatus = "pending" | "fetched" | "not_available" | "error";

export interface DrawingsStatusRow {
  doc_id: number;
  status: DrawingsStatus;
  page_count: number | null;
  source: string | null;
  updated_at: string | null;
}

export interface DrawingPage {
  doc_id: number;
  page: number;
  source: string | null;
  path: string;
  width: number | null;
  height: number | null;
  fetched_at: string | null;
}

export interface DocumentView extends DocumentDetail {
  tags: TagScore[];
  fulltext: FulltextRow | null;
  drawings_status: DrawingsStatusRow | null;
  drawing_pages: DrawingPage[];
}

export function retrieveFulltextNow(docId: number): Promise<void> {
  return invoke("retrieve_fulltext_now", { docId });
}

export function retrieveDrawingsNow(docId: number): Promise<void> {
  return invoke("retrieve_drawings_now", { docId });
}

export function runRetrievalJobs(): Promise<void> {
  return invoke("run_retrieval_jobs");
}

/// `page` is 1-based for a real page, or 0 for the `FirstPageClipping`
/// thumbnail. Returns an object URL the caller must revoke when done.
export async function drawingPageUrl(docId: number, page: number): Promise<string> {
  const bytes = await invoke<number[]>("read_drawing_page", { docId, page });
  const blob = new Blob([new Uint8Array(bytes)], { type: "image/png" });
  return URL.createObjectURL(blob);
}

export function listTags(): Promise<TagRow[]> {
  return invoke("list_tags");
}

export function createTag(
  name: string,
  definition: string,
  color: string | null,
  hotkey: string | null,
): Promise<TagRow> {
  return invoke("create_tag", { name, definition, color, hotkey });
}

export function archiveTag(tagId: number): Promise<void> {
  return invoke("archive_tag", { tagId });
}

export type QueueOrdering = "import" | "uncertain";

export function reviewQueue(ordering: QueueOrdering = "import"): Promise<QueueEntry[]> {
  return invoke("review_queue", { ordering });
}

export function documentDetail(docId: number): Promise<DocumentView | null> {
  return invoke("document_detail", { docId });
}

export interface ValidateResult {
  auto_disabled_tags: string[];
}

export function validateDocument(docId: number, checkedTagIds: number[]): Promise<ValidateResult> {
  return invoke("validate_document", { docId, checkedTagIds });
}

export function skipDocument(docId: number): Promise<void> {
  return invoke("skip_document", { docId });
}

export interface PrPoint {
  threshold: number;
  precision: number | null;
  recall: number | null;
}

export interface TagMetrics {
  name: string;
  tag_id: number;
  support_total: number;
  support_pos: number;
  precision: number | null;
  recall: number | null;
  pr_curve: PrPoint[];
}

export function tagMetrics(): Promise<TagMetrics[]> {
  return invoke("tag_metrics");
}

export function retrainNow(): Promise<number> {
  return invoke("retrain_now");
}

export interface Eligibility {
  eligible: boolean;
  n_pos: number;
  n_evaluated: number;
  calibrated_threshold: number | null;
}

export function tagEligibility(tagId: number): Promise<Eligibility> {
  return invoke("tag_eligibility", { tagId });
}

export function enableAutomaticMode(tagId: number): Promise<TagRow> {
  return invoke("enable_automatic_mode", { tagId });
}

export function disableAutomaticMode(tagId: number): Promise<void> {
  return invoke("disable_automatic_mode", { tagId });
}

export type RetrievalPolicy = "never" | "on_demand" | "after_tagging_all" | "after_tagging_selected_tags";

export interface SettingsView {
  target_precision: number;
  target_recall: number;
  audit_rate: number;
  full_automation_enabled: boolean;
  fulltext_policy: RetrievalPolicy;
  drawings_policy: RetrievalPolicy;
  fulltext_policy_tag_ids: number[];
  drawings_policy_tag_ids: number[];
}

export function getSettings(): Promise<SettingsView> {
  return invoke("get_settings");
}

export function updateSettings(view: SettingsView): Promise<void> {
  return invoke("update_settings", { view });
}

export function dataDirectory(): Promise<string> {
  return invoke("data_directory");
}

export function createBackup(destPath: string): Promise<void> {
  return invoke("create_backup", { destPath });
}

export function restoreBackup(srcPath: string): Promise<void> {
  return invoke("restore_backup", { srcPath });
}

export function exportTagSchema(destPath: string): Promise<void> {
  return invoke("export_tag_schema", { destPath });
}

export function importTagSchema(srcPath: string): Promise<number> {
  return invoke("import_tag_schema", { srcPath });
}

export interface LibraryFilters {
  query: string | null;
  include_tag_ids: number[];
  exclude_tag_ids: number[];
  label_source: string | null;
  review_state: string | null;
  /// `"available" | "unavailable"`.
  fulltext_availability: string | null;
  drawings_availability: string | null;
}

export function bulkRetrieve(docIds: number[], kind: "fulltext" | "drawings"): Promise<void> {
  return invoke("bulk_retrieve", { docIds, kind });
}

export interface LibraryRow {
  id: number;
  pub_key: string;
  title: string | null;
  review_state: string;
  tags: string[];
  has_drawings: boolean;
}

export function librarySearch(filters: LibraryFilters): Promise<LibraryRow[]> {
  return invoke("library_search", { filters });
}

export interface SimilarDocument {
  id: number;
  pub_key: string;
  title: string | null;
  similarity: number;
}

export function similarDocuments(docId: number, limit: number): Promise<SimilarDocument[]> {
  return invoke("similar_documents", { docId, limit });
}

export function exportCsv(destPath: string): Promise<void> {
  return invoke("export_csv", { destPath });
}

export function exportJson(destPath: string): Promise<void> {
  return invoke("export_json", { destPath });
}

export function exportTagList(tagId: number, destPath: string): Promise<void> {
  return invoke("export_tag_list", { tagId, destPath });
}

export function exportDocuments(docIds: number[], destDir: string): Promise<number> {
  return invoke("export_documents", { docIds, destDir });
}
