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
  source: "zero_shot" | "knn" | null;
  suggested: boolean;
}

export interface DocumentView extends DocumentDetail {
  tags: TagScore[];
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

export function reviewQueue(): Promise<QueueEntry[]> {
  return invoke("review_queue");
}

export function documentDetail(docId: number): Promise<DocumentView | null> {
  return invoke("document_detail", { docId });
}

export function validateDocument(docId: number, checkedTagIds: number[]): Promise<void> {
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
