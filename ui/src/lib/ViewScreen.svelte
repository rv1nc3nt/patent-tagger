<script lang="ts">
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { tick } from "svelte";
  import {
    createAnnotation,
    deleteAnnotation,
    findDocuments,
    HIGHLIGHT_COLORS,
    retrieveDrawingsNow,
    retrieveFulltextNow,
    setDrawingRotation,
    updateAnnotation,
    viewDocument,
    type Annotation,
    type AnnotationSection,
    type Block,
    type DrawingPage as DrawingPageRow,
    type HighlightColor,
    type QueueEntry,
    type ViewDocument,
  } from "./api";
  import FigurePanel from "./FigurePanel.svelte";
  import { espacenetUrl } from "./espacenet";
  import HighlightedText, { type FindMatch } from "./HighlightedText.svelte";

  let { docId = $bindable(null) }: { docId: number | null } = $props();

  let doc = $state<ViewDocument | null>(null);
  let error = $state("");

  // Document picker.
  let pickerQuery = $state("");
  let pickerResults = $state<QueueEntry[]>([]);
  let pickerOpen = $state(false);
  let pickerInput = $state<HTMLInputElement | undefined>(undefined);

  // Find.
  type FindScope = "all" | AnnotationSection;
  let findQuery = $state("");
  let findScope = $state<FindScope>("all");
  let currentMatch = $state(0);
  let findInput = $state<HTMLInputElement | undefined>(undefined);

  // Highlighting.
  let lastColor = $state<HighlightColor>("yellow");
  let selection = $state<{ section: AnnotationSection; start: number; end: number; rect: DOMRect } | null>(null);
  let editing = $state<{ id: number; comment: string; color: HighlightColor; rect: DOMRect } | null>(null);
  let commentInput = $state<HTMLTextAreaElement | undefined>(undefined);
  let flashing = $state<string | null>(null);

  let content = $state<HTMLElement | undefined>(undefined);
  let retrievingFulltext = $state(false);
  let retrievingDrawings = $state(false);
  let gotoParagraph = $state("");

  // Right panel: figures or highlights, resizable by its left edge.
  type SideTab = "figures" | "highlights";
  let sideTab = $state<SideTab>("highlights");
  let sideWidth = $state(380);
  let figurePage = $state(0);
  let figurePanel = $state<FigurePanel | undefined>(undefined);

  const figurePages = $derived((doc?.drawing_pages ?? []).filter((p) => p.page >= 1));

  const sectionText = $derived<Record<AnnotationSection, string>>({
    title: doc?.title ?? "",
    abstract: doc?.abstract_text ?? "",
    description: doc?.fulltext?.description ?? "",
    claims: doc?.fulltext?.claims ?? "",
  });

  const fulltextShown = $derived(
    doc?.fulltext?.status === "fetched" || doc?.fulltext?.status === "non_english_only",
  );

  const drawnAnnotations = $derived(
    (section: AnnotationSection) => (doc?.annotations ?? []).filter((a) => a.section === section && !a.detached),
  );

  const headings = $derived((doc?.description_blocks ?? []).filter((b) => b.kind === "heading"));
  const claimsByNumber = $derived(
    new Map((doc?.claim_blocks ?? []).filter((b) => b.label).map((b) => [Number(b.label), b])),
  );

  // --- Loading -------------------------------------------------------------

  async function load(id: number) {
    error = "";
    try {
      doc = await viewDocument(id);
      figurePage = 0;
      sideTab = doc && doc.drawing_pages.some((p) => p.page >= 1) ? "figures" : "highlights";
      selection = null;
      editing = null;
      currentMatch = 0;
      content?.scrollTo({ top: 0 });
    } catch (err) {
      error = String(err);
    }
  }

  $effect(() => {
    if (docId !== null) load(docId);
    else doc = null;
  });

  async function searchPicker() {
    try {
      pickerResults = await findDocuments(pickerQuery, 30);
    } catch (err) {
      error = String(err);
    }
  }

  $effect(() => {
    void pickerQuery;
    const timer = setTimeout(searchPicker, 150);
    return () => clearTimeout(timer);
  });

  function pick(entry: QueueEntry) {
    pickerOpen = false;
    pickerQuery = "";
    docId = entry.id;
  }

  async function handleRetrieveFulltext() {
    if (!doc) return;
    retrievingFulltext = true;
    error = "";
    try {
      await retrieveFulltextNow(doc.id);
      await load(doc.id);
    } catch (err) {
      error = String(err);
    } finally {
      retrievingFulltext = false;
    }
  }

  async function handleRetrieveDrawings() {
    if (!doc) return;
    retrievingDrawings = true;
    error = "";
    try {
      await retrieveDrawingsNow(doc.id);
      await load(doc.id);
      if (figurePages.length > 0) sideTab = "figures";
    } catch (err) {
      error = String(err);
    } finally {
      retrievingDrawings = false;
    }
  }

  // --- Figures ---------------------------------------------------------------

  async function handleRotate(page: DrawingPageRow, rotation: number) {
    if (!doc) return;
    const previous = page.rotation;
    page.rotation = rotation;
    try {
      await setDrawingRotation(doc.id, page.page, rotation);
    } catch (err) {
      page.rotation = previous;
      error = String(err);
    }
  }

  function showFigure(index: number) {
    figurePage = index;
    sideTab = "figures";
  }

  function startResize(e: PointerEvent) {
    e.preventDefault();
    const startX = e.clientX;
    const startWidth = sideWidth;
    const move = (ev: PointerEvent) => {
      const max = Math.max(300, window.innerWidth * 0.6);
      sideWidth = Math.round(Math.max(260, Math.min(max, startWidth + startX - ev.clientX)));
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }

  // --- Navigation ----------------------------------------------------------

  async function scrollToSelector(selector: string, flashKey?: string) {
    await tick();
    const el = content?.querySelector(selector);
    if (!el) return;
    el.scrollIntoView({ block: "center", behavior: "smooth" });
    if (flashKey) {
      flashing = flashKey;
      setTimeout(() => {
        if (flashing === flashKey) flashing = null;
      }, 1200);
    }
  }

  const goToSection = (id: string) => scrollToSelector(`#view-${id}`);
  const goToBlock = (section: string, block: Block) =>
    scrollToSelector(`[data-block="${section}-${block.start}"]`, `${section}-${block.start}`);

  function goToClaim(n: number) {
    const block = claimsByNumber.get(n);
    if (block) goToBlock("claims", block);
  }

  function goToAnnotation(a: Annotation) {
    if (a.detached) return;
    scrollToSelector(`[data-hl~="${a.id}"]`, `hl-${a.id}`);
  }

  function handleGotoParagraph(e: SubmitEvent) {
    e.preventDefault();
    const n = Number(gotoParagraph);
    const block = doc?.description_blocks.find((b) => b.label !== null && Number(b.label) === n);
    if (block) goToBlock("description", block);
    else error = `No paragraph [${gotoParagraph}] in the description.`;
  }

  // --- Find ----------------------------------------------------------------

  const matches = $derived.by(() => {
    const q = findQuery.trim().toLowerCase();
    if (q.length < 2 || !doc) return [] as FindMatch[];
    const out: FindMatch[] = [];
    const sections: AnnotationSection[] =
      findScope === "all" ? ["title", "abstract", "description", "claims"] : [findScope];
    for (const section of sections) {
      if ((section === "description" || section === "claims") && !fulltextShown) continue;
      const hay = sectionText[section].toLowerCase();
      let from = 0;
      for (;;) {
        const at = hay.indexOf(q, from);
        if (at < 0) break;
        out.push({ section, start: at, end: at + q.length, index: out.length });
        from = at + q.length;
      }
    }
    return out;
  });

  const matchesIn = $derived((section: string) => matches.filter((m) => m.section === section));

  $effect(() => {
    void matches;
    currentMatch = 0;
    if (matches.length > 0) scrollToSelector(`[data-match="0"]`);
  });

  function stepMatch(delta: number) {
    if (matches.length === 0) return;
    currentMatch = (currentMatch + delta + matches.length) % matches.length;
    scrollToSelector(`[data-match="${currentMatch}"]`);
  }

  function handleFindKey(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      stepMatch(e.shiftKey ? -1 : 1);
    } else if (e.key === "Escape") {
      findQuery = "";
      findInput?.blur();
    }
  }

  // --- Selection and highlights ---------------------------------------------

  /// Maps a DOM selection end point to (section, offset) via the
  /// `data-offset` of the span holding it.
  function pointOffset(node: Node, offset: number): { section: AnnotationSection; offset: number } | null {
    let seg: HTMLElement | null = null;
    let at = 0;
    if (node.nodeType === Node.TEXT_NODE) {
      seg = node.parentElement?.closest<HTMLElement>("[data-offset]") ?? null;
      at = offset;
    } else {
      const el = node as HTMLElement;
      const child = el.childNodes[offset];
      if (child instanceof HTMLElement) {
        seg = child.matches("[data-offset]") ? child : child.querySelector<HTMLElement>("[data-offset]");
      } else if (child) {
        seg = child.parentElement?.closest<HTMLElement>("[data-offset]") ?? null;
      } else {
        const all = el.querySelectorAll<HTMLElement>("[data-offset]");
        seg = all[all.length - 1] ?? null;
        at = seg?.textContent?.length ?? 0;
      }
    }
    const sectionEl = seg?.closest<HTMLElement>("[data-section]");
    if (!seg || !sectionEl) return null;
    return {
      section: sectionEl.dataset.section as AnnotationSection,
      offset: Number(seg.dataset.offset) + at,
    };
  }

  function handleMouseUp() {
    // Let the browser settle the selection first.
    setTimeout(readSelection, 0);
  }

  function readSelection() {
    const sel = window.getSelection();
    if (!sel || sel.isCollapsed || sel.rangeCount === 0 || !content) {
      selection = null;
      return;
    }
    const range = sel.getRangeAt(0);
    if (!content.contains(range.commonAncestorContainer)) return;
    const a = pointOffset(range.startContainer, range.startOffset);
    let b = pointOffset(range.endContainer, range.endOffset);
    if (!a) {
      selection = null;
      return;
    }
    if (!b || b.section !== a.section) {
      // A selection running past the end of a section (a triple click,
      // say) is cut at the last span of the first section it covers.
      const spans = [...content.querySelectorAll<HTMLElement>(`[data-section="${a.section}"] [data-offset]`)].filter(
        (s) => range.intersectsNode(s),
      );
      const last = spans[spans.length - 1];
      if (!last) return;
      b = { section: a.section, offset: Number(last.dataset.offset) + (last.textContent?.length ?? 0) };
    }
    const text = sectionText[a.section];
    let start = Math.min(a.offset, b.offset);
    let end = Math.max(a.offset, b.offset);
    while (start < end && /\s/.test(text[start])) start++;
    while (end > start && /\s/.test(text[end - 1])) end--;
    if (start >= end) {
      selection = null;
      return;
    }
    editing = null;
    selection = { section: a.section, start, end, rect: range.getBoundingClientRect() };
  }

  async function highlightSelection(color: HighlightColor, withComment: boolean) {
    if (!doc || !selection) return;
    const { section, start, end, rect } = selection;
    error = "";
    try {
      const list = await createAnnotation(doc.id, section, start, end, null, color);
      doc.annotations = list;
      lastColor = color;
      selection = null;
      window.getSelection()?.removeAllRanges();
      if (withComment) {
        const created = list
          .filter((x) => x.section === section && x.start === start && x.end === end)
          .sort((x, y) => y.id - x.id)[0];
        if (created) openEditor(created, rect);
      }
    } catch (err) {
      error = String(err);
    }
  }

  async function openEditor(a: Annotation, rect: DOMRect) {
    selection = null;
    editing = { id: a.id, comment: a.comment ?? "", color: a.color, rect };
    await tick();
    commentInput?.focus();
  }

  function openEditorById(id: number, rect: DOMRect) {
    const a = doc?.annotations.find((x) => x.id === id);
    if (a) openEditor(a, rect);
  }

  async function saveEditor() {
    if (!doc || !editing) return;
    error = "";
    try {
      doc.annotations = await updateAnnotation(editing.id, editing.comment, editing.color);
      lastColor = editing.color;
      editing = null;
    } catch (err) {
      error = String(err);
    }
  }

  async function removeAnnotation(id: number) {
    if (!doc) return;
    error = "";
    try {
      doc.annotations = await deleteAnnotation(doc.id, id);
      if (editing?.id === id) editing = null;
    } catch (err) {
      error = String(err);
    }
  }

  function handleEditorKey(e: KeyboardEvent) {
    if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      saveEditor();
    } else if (e.key === "Escape") {
      editing = null;
    }
  }

  /// Keeps a floating panel on screen: below the anchor, or above it when
  /// there is no room below.
  function popoverStyle(rect: DOMRect, height: number): string {
    const width = 300;
    const left = Math.max(8, Math.min(rect.left, window.innerWidth - width - 8));
    const below = rect.bottom + 6;
    const top = below + height > window.innerHeight ? Math.max(8, rect.top - height - 6) : below;
    return `left: ${left}px; top: ${top}px; width: ${width}px;`;
  }

  function handleKeydown(e: KeyboardEvent) {
    const target = e.target as HTMLElement | null;
    const typing = target?.tagName === "INPUT" || target?.tagName === "TEXTAREA" || target?.tagName === "SELECT";
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "f") {
      e.preventDefault();
      findInput?.focus();
      findInput?.select();
      return;
    }
    if (typing) return;
    if (e.key === "/") {
      e.preventDefault();
      findInput?.focus();
    } else if (e.key === "F3") {
      e.preventDefault();
      stepMatch(e.shiftKey ? -1 : 1);
    } else if (e.key === "Escape") {
      selection = null;
      editing = null;
    } else if (sideTab === "figures" && figurePanel && (e.key === "r" || e.key === "R")) {
      e.preventDefault();
      figurePanel.turn(e.shiftKey ? -90 : 90);
    } else if (sideTab === "figures" && figurePanel && (e.key === "[" || e.key === "]")) {
      e.preventDefault();
      figurePanel.step(e.key === "]" ? 1 : -1);
    } else if (selection && (e.key === "h" || e.key === "H")) {
      e.preventDefault();
      highlightSelection(lastColor, false);
    } else if (selection && (e.key === "c" || e.key === "C")) {
      // Otherwise the "c" lands in the comment box, focused meanwhile.
      e.preventDefault();
      highlightSelection(lastColor, true);
    }
  }

  function snippet(block: Block, text: string, max = 60): string {
    const raw = text.slice(block.start, block.end).replace(/^\s*\d+\s*[.)]\s*/, "");
    return raw.length > max ? `${raw.slice(0, max)}…` : raw;
  }

  const espacenetLink = $derived(doc ? espacenetUrl(doc.pub_key, doc.kind_codes) : null);
  const annotationCount = $derived(doc?.annotations.length ?? 0);
</script>

<svelte:window onkeydown={handleKeydown} />

<div class="view">
  <div class="topbar">
    <div class="picker">
      <input
        bind:this={pickerInput}
        bind:value={pickerQuery}
        placeholder="Open a document: number or title"
        onfocus={() => (pickerOpen = true)}
        onblur={() => setTimeout(() => (pickerOpen = false), 150)}
        onkeydown={(e) => {
          if (e.key === "Enter" && pickerResults[0]) pick(pickerResults[0]);
          if (e.key === "Escape") pickerInput?.blur();
        }}
      />
      {#if pickerOpen && pickerResults.length > 0}
        <ul class="picker-results">
          {#each pickerResults as entry (entry.id)}
            <li>
              <button onmousedown={(e) => e.preventDefault()} onclick={() => pick(entry)}>
                <strong>{entry.pub_key}</strong> {entry.title ?? "…"}
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    </div>

    {#if doc}
      <div class="find">
        <input
          bind:this={findInput}
          bind:value={findQuery}
          placeholder="Find in document (Ctrl+F)"
          onkeydown={handleFindKey}
        />
        <select bind:value={findScope} title="Where to search">
          <option value="all">Everywhere</option>
          <option value="abstract">Abstract</option>
          {#if fulltextShown}
            <option value="description">Description</option>
            <option value="claims">Claims</option>
          {/if}
        </select>
        <span class="count">
          {#if findQuery.trim().length >= 2}
            {matches.length === 0 ? "No match" : `${currentMatch + 1} / ${matches.length}`}
          {/if}
        </span>
        <button onclick={() => stepMatch(-1)} disabled={matches.length === 0} title="Previous match (Shift+Enter)">↑</button>
        <button onclick={() => stepMatch(1)} disabled={matches.length === 0} title="Next match (Enter)">↓</button>
      </div>
    {/if}
  </div>

  {#if error}<p class="error-banner">{error}</p>{/if}

  {#if !doc}
    <p class="empty">
      Open a document with the box above, or with “View” in the Review and Library tabs.
    </p>
  {:else}
    <div class="panes" style:grid-template-columns="200px minmax(0, 1fr) {sideWidth}px">
      <nav class="outline" aria-label="Outline">
        <button class="section-link" onclick={() => goToSection("metadata")}>Metadata</button>
        <button class="section-link" onclick={() => goToSection("abstract")}>Abstract</button>
        <button class="section-link" onclick={() => goToSection("description")}>Description</button>
        {#if fulltextShown && doc.description_blocks.length > 0}
          {#each headings as h (h.start)}
            <button class="sub" onclick={() => goToBlock("description", h)}>{snippet(h, sectionText.description, 40)}</button>
          {/each}
          <form class="goto" onsubmit={handleGotoParagraph}>
            <label>¶ <input bind:value={gotoParagraph} inputmode="numeric" placeholder="0012" size="5" /></label>
            <button type="submit">Go</button>
          </form>
        {/if}
        <button class="section-link" onclick={() => goToSection("claims")}>Claims</button>
        {#if fulltextShown}
          {#each doc.claim_blocks.filter((b) => b.label) as c (c.start)}
            <button
              class="sub claim-entry"
              class:independent={c.depends_on.length === 0}
              class:dependent={c.depends_on.length > 0}
              title={c.depends_on.length > 0 ? `Depends on claim ${c.depends_on.join(", ")}` : "Independent claim"}
              onclick={() => goToBlock("claims", c)}
            >
              <span class="claim-no">{c.label}</span>
              {snippet(c, sectionText.claims, 36)}
            </button>
          {/each}
        {/if}
        <button class="section-link" onclick={() => goToSection("drawings")}>Drawings</button>
      </nav>

      <!-- Selecting text anywhere in the document offers highlighting. -->
      <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
      <article class="content" bind:this={content} onmouseup={handleMouseUp}>
        <section id="view-metadata">
          <h2>{doc.pub_key}{#if doc.kind_codes.length > 0}<span class="kinds"> {doc.kind_codes.join(", ")}</span>{/if}</h2>
          <p class="title" data-section="title">
            <HighlightedText
              text={sectionText.title}
              start={0}
              end={sectionText.title.length}
              annotations={drawnAnnotations("title")}
              matches={matchesIn("title")}
              {currentMatch}
              onAnnotation={openEditorById}
            />
          </p>
          <dl class="meta">
            <dt>Applicants</dt><dd>{doc.applicants.join("; ") || "—"}</dd>
            <dt>Publication date</dt><dd>{doc.publication_date ?? "—"}</dd>
            <dt>Application</dt><dd>{doc.application_number ?? "—"}</dd>
            <dt>Family</dt><dd>{doc.family_id ?? "—"}</dd>
            <dt>CPC</dt><dd>{doc.cpc.join(", ") || "—"}</dd>
            <dt>IPC</dt><dd>{doc.ipc.join(", ") || "—"}</dd>
            <dt>Tags</dt><dd>{doc.tags.join(", ") || "None"}</dd>
          </dl>
          {#if espacenetLink}
            <button class="link" onclick={() => openUrl(espacenetLink)}>Open in Espacenet</button>
          {/if}
        </section>

        <section id="view-abstract">
          <h3>Abstract</h3>
          {#if doc.abstract_source}<p class="source-note">Source: {doc.abstract_source}</p>{/if}
          {#if sectionText.abstract}
            <p class="text" data-section="abstract">
              <HighlightedText
                text={sectionText.abstract}
                start={0}
                end={sectionText.abstract.length}
                annotations={drawnAnnotations("abstract")}
                matches={matchesIn("abstract")}
                {currentMatch}
                onAnnotation={openEditorById}
              />
            </p>
          {:else}
            <p class="note">No English abstract.</p>
          {/if}
        </section>

        {#if !fulltextShown}
          <section id="view-description">
            <h3>Description and claims</h3>
            {#if doc.fulltext?.status === "not_available"}
              <p class="note">Full text not available in OPS.</p>
            {:else}
              <p class="note">
                {doc.fulltext?.status === "error" ? "Full text retrieval failed." : "Full text not retrieved."}
              </p>
              <button onclick={handleRetrieveFulltext} disabled={retrievingFulltext}>
                {retrievingFulltext ? "Retrieving…" : "Retrieve full text now"}
              </button>
            {/if}
          </section>
          <span id="view-claims"></span>
        {:else}
          {#each [{ id: "description" as const, title: "Description", blocks: doc.description_blocks }, { id: "claims" as const, title: "Claims", blocks: doc.claim_blocks }] as part (part.id)}
            <section id="view-{part.id}">
              <h3>{part.title}</h3>
              <p class="source-note">
                Source: {doc.fulltext?.source}
                {#if doc.fulltext?.status === "non_english_only"}· non-English text (language: {doc.fulltext.lang}){/if}
              </p>
              {#if part.blocks.length === 0}
                <p class="note">{part.title} not available for this publication.</p>
              {/if}
              <div data-section={part.id}>
                {#each part.blocks as block (block.start)}
                  <p
                    class="text block {block.kind}"
                    class:flash={flashing === `${part.id}-${block.start}`}
                    data-block="{part.id}-{block.start}"
                  >
                    <HighlightedText
                      text={sectionText[part.id]}
                      start={block.start}
                      end={block.end}
                      annotations={drawnAnnotations(part.id)}
                      matches={matchesIn(part.id)}
                      {currentMatch}
                      claimRefs={block.claim_refs}
                      onClaim={goToClaim}
                      onAnnotation={openEditorById}
                    />
                  </p>
                {/each}
              </div>
            </section>
          {/each}
        {/if}

        <section id="view-drawings">
          <h3>Drawings</h3>
          {#if doc.drawings_status?.status === "fetched"}
            <p class="source-note">
              Source: {doc.drawings_status.source} · {doc.drawings_status.page_count} page(s)
            </p>
            <div class="page-chips">
              {#each figurePages as page, i (page.page)}
                <button onclick={() => showFigure(i)} title="Show page {page.page} in the Figures panel">
                  Page {page.page}
                </button>
              {/each}
            </div>
          {:else if doc.drawings_status?.status === "not_available"}
            <p class="note">This publication has no drawings.</p>
          {:else}
            <p class="note">
              {doc.drawings_status?.status === "error" ? "Drawings retrieval failed." : "Drawings not retrieved."}
            </p>
            <button onclick={handleRetrieveDrawings} disabled={retrievingDrawings}>
              {retrievingDrawings ? "Retrieving…" : "Retrieve drawings now"}
            </button>
          {/if}
        </section>
      </article>

      <aside class="side">
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div class="resize-handle" onpointerdown={startResize} title="Drag to resize"></div>
        <div class="side-tabs">
          <button class:active={sideTab === "figures"} onclick={() => (sideTab = "figures")}>
            Figures{figurePages.length > 0 ? ` (${figurePages.length})` : ""}
          </button>
          <button class:active={sideTab === "highlights"} onclick={() => (sideTab = "highlights")}>
            Highlights ({annotationCount})
          </button>
        </div>
        {#if sideTab === "figures"}
          {#if figurePages.length > 0}
            <FigurePanel
              bind:this={figurePanel}
              docId={doc.id}
              pages={figurePages}
              bind:current={figurePage}
              onRotate={handleRotate}
            />
          {:else if doc.drawings_status?.status === "not_available"}
            <p class="note">This publication has no drawings.</p>
          {:else}
            <p class="note">
              {doc.drawings_status?.status === "error" ? "Drawings retrieval failed." : "Drawings not retrieved."}
            </p>
            <button onclick={handleRetrieveDrawings} disabled={retrievingDrawings}>
              {retrievingDrawings ? "Retrieving…" : "Retrieve drawings now"}
            </button>
          {/if}
        {:else}
          <div class="highlights">
            {#if annotationCount === 0}
              <p class="note">Select text, then pick a colour, or press H (highlight) or C (highlight with a comment).</p>
            {/if}
            <ul>
              {#each doc.annotations as a (a.id)}
                <li class:detached={a.detached} class:flash={flashing === `hl-${a.id}`}>
                  <button class="hl-entry" onclick={() => goToAnnotation(a)} disabled={a.detached}>
                    <span class="swatch" style:background="var(--hl-{a.color})"></span>
                    <span class="location">{a.location}</span>
                    <span class="quote">“{a.quote}”</span>
                    {#if a.comment}<span class="comment">{a.comment}</span>{/if}
                    {#if a.detached}<span class="note">Passage no longer in the text.</span>{/if}
                  </button>
                  <div class="hl-actions">
                    <button class="link" onclick={(e) => openEditor(a, (e.currentTarget as HTMLElement).getBoundingClientRect())}>
                      Edit
                    </button>
                    <button class="link danger" onclick={() => removeAnnotation(a.id)}>Delete</button>
                  </div>
                </li>
              {/each}
            </ul>
          </div>
        {/if}
      </aside>
    </div>
  {/if}
</div>

{#if selection}
  <div class="popover toolbar" style={popoverStyle(selection.rect, 44)}>
    {#each HIGHLIGHT_COLORS as color (color)}
      <button
        class="swatch-button"
        class:last={color === lastColor}
        style:background="var(--hl-{color})"
        title="Highlight in {color}{color === lastColor ? ' (H)' : ''}"
        aria-label="Highlight in {color}"
        onmousedown={(e) => e.preventDefault()}
        onclick={() => highlightSelection(color, false)}
      ></button>
    {/each}
    <button onmousedown={(e) => e.preventDefault()} onclick={() => highlightSelection(lastColor, true)} title="Highlight and comment (C)">
      Comment…
    </button>
  </div>
{/if}

{#if editing}
  <div class="popover editor" style={popoverStyle(editing.rect, 190)}>
    <div class="colors">
      {#each HIGHLIGHT_COLORS as color (color)}
        <button
          class="swatch-button"
          class:last={color === editing.color}
          style:background="var(--hl-{color})"
          aria-label={color}
          onclick={() => editing && (editing.color = color)}
        ></button>
      {/each}
    </div>
    <textarea
      bind:this={commentInput}
      bind:value={editing.comment}
      rows="3"
      placeholder="Comment (optional)"
      onkeydown={handleEditorKey}
    ></textarea>
    <div class="editor-actions">
      <button class="link danger" onclick={() => editing && removeAnnotation(editing.id)}>Delete</button>
      <span class="spacer"></span>
      <button onclick={() => (editing = null)}>Cancel</button>
      <button class="primary" onclick={saveEditor} title="Save (Ctrl+Enter)">Save</button>
    </div>
  </div>
{/if}

<style>
  .view {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    padding: 1rem 1.5rem;
    max-width: 1400px;
    margin: 0 auto;
  }
  .topbar {
    display: flex;
    gap: 1rem;
    align-items: center;
    flex-wrap: wrap;
  }
  .picker {
    position: relative;
    flex: 1 1 280px;
    max-width: 420px;
  }
  .picker input {
    width: 100%;
    padding: 0.4rem 0.6rem;
  }
  .picker-results {
    position: absolute;
    z-index: 20;
    top: 100%;
    left: 0;
    right: 0;
    margin: 2px 0 0;
    padding: 0.25rem;
    list-style: none;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 6px;
    max-height: 60vh;
    overflow: auto;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.15);
  }
  .picker-results button {
    width: 100%;
    text-align: left;
    border: none;
    background: transparent;
    padding: 0.3rem 0.4rem;
    font-size: 0.9rem;
  }
  .picker-results button:hover {
    background: var(--bg-alt);
  }
  .find {
    display: flex;
    gap: 0.35rem;
    align-items: center;
  }
  .find input {
    padding: 0.4rem 0.6rem;
    width: 240px;
  }
  .find select {
    padding: 0.35rem;
  }
  .find .count {
    min-width: 5.5rem;
    font-size: 0.85rem;
    color: var(--muted);
    font-variant-numeric: tabular-nums;
  }
  .find button {
    padding: 0.3rem 0.55rem;
  }
  .error-banner {
    color: var(--danger);
    margin: 0;
  }
  .empty,
  .note {
    color: var(--muted);
  }
  .note {
    font-size: 0.85rem;
  }
  .panes {
    display: grid;
    gap: 1rem;
    height: calc(100svh - 8.5rem);
  }
  .outline,
  .content,
  .highlights {
    overflow-y: auto;
    min-height: 0;
  }
  .side {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    min-height: 0;
    min-width: 0;
    padding-left: 0.5rem;
  }
  .resize-handle {
    position: absolute;
    left: -0.5rem;
    top: 0;
    bottom: 0;
    width: 0.6rem;
    cursor: col-resize;
    border-left: 1px solid var(--border);
    margin-left: 0.25rem;
  }
  .resize-handle:hover {
    border-left: 2px solid var(--accent);
  }
  .side-tabs {
    display: flex;
    gap: 0.75rem;
    border-bottom: 1px solid var(--border);
    flex: none;
  }
  .side-tabs button {
    background: transparent;
    border: none;
    border-bottom: 2px solid transparent;
    border-radius: 0;
    padding: 0.3rem 0.1rem;
    font-weight: 600;
    color: var(--muted);
  }
  .side-tabs button.active {
    color: var(--text-h);
    border-bottom-color: var(--accent);
  }
  .page-chips {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem;
  }
  .page-chips button {
    font-size: 0.85rem;
    padding: 0.2rem 0.6rem;
  }
  .outline {
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding-right: 0.25rem;
  }
  .outline button {
    border: none;
    background: transparent;
    text-align: left;
    padding: 0.2rem 0.4rem;
    border-radius: 4px;
  }
  .outline button:hover {
    background: var(--bg-alt);
  }
  .section-link {
    font-weight: 600;
    color: var(--text-h);
    margin-top: 0.4rem;
  }
  .sub {
    font-size: 0.8rem;
    color: var(--muted);
    padding-left: 1rem !important;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .claim-entry.independent {
    color: var(--text-h);
    font-weight: 600;
  }
  .claim-entry.dependent {
    padding-left: 1.75rem !important;
  }
  .claim-no {
    display: inline-block;
    min-width: 1.6rem;
    font-variant-numeric: tabular-nums;
  }
  .goto {
    display: flex;
    gap: 0.25rem;
    align-items: center;
    padding-left: 1rem;
    font-size: 0.8rem;
    color: var(--muted);
  }
  .goto input {
    padding: 0.1rem 0.3rem;
    font-size: 0.8rem;
  }
  .goto button {
    font-size: 0.8rem;
    padding: 0.1rem 0.4rem !important;
    border: 1px solid var(--border) !important;
  }
  .content {
    padding: 0 1rem 40vh 0.25rem;
    line-height: 1.6;
  }
  .content section {
    margin-bottom: 1.75rem;
  }
  .content h3 {
    border-bottom: 1px solid var(--border);
    padding-bottom: 0.25rem;
    margin-bottom: 0.5rem;
  }
  .kinds {
    font-weight: 400;
    color: var(--muted);
    font-size: 0.9rem;
  }
  .title {
    font-size: 1.1rem;
    font-weight: 600;
    color: var(--text-h);
    margin: 0.25rem 0 0.75rem;
  }
  .meta {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 0.15rem 1rem;
    font-size: 0.9rem;
    margin: 0 0 0.5rem;
  }
  .meta dt {
    color: var(--muted);
  }
  .meta dd {
    margin: 0;
  }
  .source-note {
    font-size: 0.8rem;
    color: var(--muted);
    margin: 0 0 0.5rem;
  }
  .text {
    white-space: pre-wrap;
    margin: 0 0 0.6rem;
  }
  .block {
    border-radius: 4px;
    transition: background-color 0.4s;
  }
  .block.heading {
    font-weight: 600;
    color: var(--text-h);
    margin-top: 1rem;
  }
  .block.claim {
    padding-left: 0.5rem;
    border-left: 2px solid var(--border);
  }
  .flash {
    background-color: var(--bg-alt);
    outline: 2px solid var(--accent);
  }
  .link {
    background: none;
    border: none;
    padding: 0;
    color: var(--accent);
    font-size: 0.8rem;
    text-decoration: underline;
  }
  .link.danger {
    color: var(--danger);
  }
  .highlights ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .highlights li {
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 0.4rem 0.5rem;
  }
  .highlights li.detached {
    opacity: 0.7;
    border-style: dashed;
  }
  .hl-entry {
    display: grid;
    grid-template-columns: 0.75rem 1fr;
    gap: 0.15rem 0.4rem;
    width: 100%;
    text-align: left;
    border: none;
    background: transparent;
    padding: 0;
    font-size: 0.85rem;
  }
  .hl-entry > :not(.swatch) {
    grid-column: 2;
  }
  .swatch {
    width: 0.75rem;
    height: 0.75rem;
    border-radius: 2px;
    margin-top: 0.25rem;
    grid-row: 1;
  }
  .location {
    font-weight: 600;
    color: var(--text-h);
    grid-row: 1;
  }
  .quote {
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .comment {
    white-space: pre-wrap;
    color: var(--text-h);
    border-left: 2px solid var(--accent);
    padding-left: 0.4rem;
  }
  .hl-actions {
    display: flex;
    gap: 0.75rem;
    justify-content: flex-end;
    margin-top: 0.2rem;
  }
  .popover {
    position: fixed;
    z-index: 50;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 8px;
    box-shadow: 0 6px 20px rgba(0, 0, 0, 0.2);
    padding: 0.4rem;
  }
  .toolbar {
    display: flex;
    gap: 0.35rem;
    align-items: center;
    width: auto !important;
  }
  .swatch-button {
    width: 1.6rem;
    height: 1.6rem;
    padding: 0;
    border-radius: 50%;
    border: 2px solid var(--border);
  }
  .swatch-button.last {
    border-color: var(--text);
  }
  .editor {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
  }
  .colors {
    display: flex;
    gap: 0.35rem;
  }
  .editor textarea {
    width: 100%;
    padding: 0.35rem;
    resize: vertical;
  }
  .editor-actions {
    display: flex;
    gap: 0.4rem;
    align-items: center;
  }
  .spacer {
    flex: 1;
  }
  .primary {
    background: var(--accent);
    color: var(--bg);
    border-color: var(--accent);
  }
</style>
