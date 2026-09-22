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
