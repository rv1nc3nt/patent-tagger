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

export interface SearchFields {
  name: string;
  applicant: string;
  country: string | null;
  year_from: number | null;
  year_to: number | null;
}

export interface SavedSearch {
  id: number;
  name: string;
  applicant: string;
  country: string | null;
  year_from: number | null;
  year_to: number | null;
  query: string;
  total_results: number | null;
  next_start: number;
  imported: number;
  created_at: string;
  last_run_at: string | null;
  exhausted: boolean;
  results_read: number;
  capped: boolean;
}

export interface BatchReport {
  search: SavedSearch | null;
  range: [number, number] | null;
  scanned: number;
  families: number;
  known_families: string[];
  duplicates: string[];
  imported: string[];
}

export function listSearches(): Promise<SavedSearch[]> {
  return invoke("list_searches");
}

export function previewSearchQuery(fields: SearchFields): Promise<string> {
  return invoke("preview_search_query", { fields });
}

export function createSearch(fields: SearchFields): Promise<SavedSearch> {
  return invoke("create_search", { fields });
}

export function deleteSearch(searchId: number): Promise<void> {
  return invoke("delete_search", { searchId });
}

export function restartSearch(searchId: number): Promise<void> {
  return invoke("restart_search", { searchId });
}

export function fetchSearchBatch(searchId: number): Promise<BatchReport> {
  return invoke("fetch_search_batch", { searchId });
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
  parent_id: number | null;
  version: number;
  archived: boolean;
  threshold: number | null;
  neg_threshold: number | null;
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
  parent_id: number | null;
  color: string | null;
  hotkey: string | null;
  score: number | null;
  source: "blend" | "lr" | "knn" | "zero_shot" | null;
  model_version: string;
  suggested: boolean;
  /// SPEC 8: "confident automatic tags are shown as filled."
  automatic: boolean;
  /// SPEC 8: "uncertain tags...are highlighted."
  uncertain: boolean;
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

export function listArchivedTags(): Promise<TagRow[]> {
  return invoke("list_archived_tags");
}

export interface TagFields {
  name: string;
  definition: string;
  parentId: number | null;
  color: string | null;
  hotkey: string | null;
}

export function createTag(fields: TagFields): Promise<TagRow> {
  return invoke("create_tag", { ...fields });
}

export function updateTag(tagId: number, fields: TagFields, bumpVersion: boolean): Promise<TagRow> {
  return invoke("update_tag", { tagId, ...fields, bumpVersion });
}

export function archiveTag(tagId: number): Promise<void> {
  return invoke("archive_tag", { tagId });
}

export interface RestoredTag {
  tag: TagRow;
  /// The tag's former hotkey, when another active tag took it meanwhile.
  hotkey_cleared: string | null;
}

export function unarchiveTag(tagId: number): Promise<RestoredTag> {
  return invoke("unarchive_tag", { tagId });
}

export interface TagStats {
  tag_id: number;
  human_pos: number;
  human_neg: number;
  auto_pos: number;
  auto_neg: number;
  /// Reviewed documents with no label for this tag.
  unlabelled: number;
  /// Human labels written under an older tag version.
  stale: number;
}

export interface TagOverview {
  tag: TagRow;
  stats: TagStats;
  eligibility: Eligibility;
}

export function tagOverview(): Promise<TagOverview[]> {
  return invoke("tag_overview");
}

export function discardStaleLabels(tagId: number): Promise<number> {
  return invoke("discard_stale_labels", { tagId });
}

export interface TagReviewItem {
  doc_id: number;
  pub_key: string;
  title: string | null;
  previous_state: "pos" | "neg" | null;
  previous_version: number | null;
  score: number | null;
}

export function tagReviewQueue(tagId: number): Promise<TagReviewItem[]> {
  return invoke("tag_review_queue", { tagId });
}

export function labelSingleTag(docId: number, tagId: number, positive: boolean): Promise<void> {
  return invoke("label_single_tag", { docId, tagId, positive });
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
  audited_precision: number | null;
  audited_n: number;
  full_automation_precision: number | null;
  full_automation_recall: number | null;
  full_automation_n: number;
}

export function tagMetrics(): Promise<TagMetrics[]> {
  return invoke("tag_metrics");
}

export interface FullAutomationSummary {
  would_auto_complete: number;
  total: number;
  auto_completed: number;
  audited_or_complete: number;
  focused_review: number;
}

export function fullAutomationSummary(): Promise<FullAutomationSummary> {
  return invoke("full_automation_summary");
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

export interface SchemaImport {
  created: number[];
  hotkeys_dropped: string[];
  parents_dropped: string[];
}

export function importTagSchema(srcPath: string): Promise<SchemaImport> {
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

export type JobKind = "import_document" | "fulltext_retrieval" | "drawings_retrieval";

export interface JobCounts {
  kind: JobKind;
  pending: number;
  running: number;
  failed: number;
}

export interface JobDetail {
  id: number;
  kind: JobKind;
  state: string;
  doc_id: number | null;
  pub_key: string | null;
  attempts: number;
  last_error: string | null;
  updated_at: string;
}

export interface JobOverview {
  counts: JobCounts[];
  active_total: number;
  failed_total: number;
  running: JobDetail[];
  failed: JobDetail[];
  import_running: boolean;
  retrieval_running: boolean;
}

export function jobOverview(): Promise<JobOverview> {
  return invoke("job_overview");
}

export function retryJob(jobId: number): Promise<void> {
  return invoke("retry_job", { jobId });
}

export function retryFailedJobs(kind: JobKind): Promise<number> {
  return invoke("retry_failed_jobs", { kind });
}

export function cancelPendingJobs(kind: JobKind): Promise<number> {
  return invoke("cancel_pending_jobs", { kind });
}
