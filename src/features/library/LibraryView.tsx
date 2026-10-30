import { useCallback, useEffect, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { Icon } from "../../icons/Icon";
import { api, toError } from "../../ipc/api";
import type { LibraryFacets, LibraryItem, LibraryQuery, LibrarySort } from "../../ipc/types";
import { bytes, clock, MONOGRAM, relativeDate } from "../../lib/format";
import { useApp } from "../../state/store";

const ROW = 48;

const SORTS: { id: LibrarySort; label: string }[] = [
  { id: "newest", label: "Newest" },
  { id: "oldest", label: "Oldest" },
  { id: "largest", label: "Largest" },
  { id: "title", label: "Title A–Z" },
];
const KIND_LABEL: Record<string, string> = { video: "Video", audio: "Audio", image: "Image", gallery: "Gallery" };
const PLATFORM_LABEL: Record<string, string> = {
  youtube: "YouTube", tiktok: "TikTok", douyin: "Douyin", bilibili: "Bilibili", instagram: "Instagram", facebook: "Facebook",
  twitter: "X", pinterest: "Pinterest", kuaishou: "Kuaishou", rednote: "RedNote", suno: "Suno", other: "Other",
};

function Chip({ on, onClick, children }: { on: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      aria-pressed={on}
      onClick={onClick}
      className={`h-6 rounded-[3px] border px-2 text-xs transition-colors duration-[90ms] ${on ? "border-[var(--amber-line)] bg-[var(--amber-tint)] text-fg-1" : "border-line-2 text-fg-3 hover:border-line-3 hover:text-fg-1"}`}
    >
      {children}
    </button>
  );
}

export default function LibraryView() {
  const [text, setText] = useState("");
  const [platform, setPlatform] = useState<string | null>(null);
  const [kind, setKind] = useState<string | null>(null);
  const [sort, setSort] = useState<LibrarySort>("newest");
  const [items, setItems] = useState<LibraryItem[] | null>(null);
  const [facets, setFacets] = useState<LibraryFacets | null>(null);
  const [confirm, setConfirm] = useState<LibraryItem | null>(null);
  const toast = useApp((s) => s.toast);
  const completedCount = useApp((s) => s.order.filter((id) => s.tasks[id]?.state === "completed").length);
  const parent = useRef<HTMLDivElement>(null);
  const query = text;

  const load = useCallback(() => {
    const q: LibraryQuery = { text, platform, kind, sort, offset: 0 };
    api.libraryList(q).then(setItems, (e) => toast({ tone: "err", title: "Couldn't load the library", body: toError(e).message }));
    api.libraryFacets().then(setFacets, () => {});
  }, [text, platform, kind, sort, toast]);

  useEffect(() => {
    const t = setTimeout(load, 120);
    return () => clearTimeout(t);
  }, [load, completedCount]);

  const filtered = !!(text || platform || kind);

  const v = useVirtualizer({ count: items?.length ?? 0, getScrollElement: () => parent.current, estimateSize: () => ROW, overscan: 8 });

  const run = (fn: () => Promise<unknown>, okMsg?: string) =>
    fn().then(() => okMsg && toast({ tone: "info", title: okMsg }), (e) => toast({ tone: "err", title: "Action failed", body: toError(e).message }));

  const remove = async (item: LibraryItem, deleteFile: boolean) => {
    setConfirm(null);
    await run(() => api.libraryRemove(item.id, deleteFile), deleteFile ? "File deleted" : "Removed from library");
    load();
  };

  return (
    <div className="rise flex h-full flex-col px-6">
      <div className="flex items-center gap-3 pt-5 pb-4">
        <h1 className="text-lg font-semibold tracking-[-0.01em]">Library</h1>
        <span className="t-data text-fg-3">
          {items ? `${items.length}${items.length === 200 ? "+" : ""}${filtered && facets ? ` of ${facets.total}` : ""} ${(filtered ? facets?.total : items.length) === 1 ? "item" : "items"}` : ""}
        </span>
        <div className="flex-1" />
        <label className="field flex w-64 items-center gap-2">
          <Icon name="search" className="text-fg-3" />
          <input
            id="library-search"
            value={query}
            onChange={(e) => setText(e.target.value)}
            placeholder="Search title or creator"
            className="h-full min-w-0 flex-1 bg-transparent outline-none placeholder:text-fg-3"
            aria-label="Search library"
            onKeyDown={(e) => e.key === "Escape" && setText("")}
          />
        </label>
        <label className="field flex items-center gap-2 pr-1">
          <span className="t-micro">Sort</span>
          <select
            value={sort}
            onChange={(e) => setSort(e.target.value as LibrarySort)}
            className="h-full cursor-default bg-transparent text-fg-1 outline-none"
            aria-label="Sort library"
          >
            {SORTS.map((s) => <option key={s.id} value={s.id} className="bg-bg-2">{s.label}</option>)}
          </select>
        </label>
      </div>

      {facets && facets.total > 0 && (
        <div className="-mt-1 mb-3 flex flex-wrap items-center gap-1.5" role="group" aria-label="Filters">
          <Chip on={!platform && !kind} onClick={() => { setPlatform(null); setKind(null); }}>All <span className="mono text-fg-4">{facets.total}</span></Chip>
          <span className="mx-1 h-4 w-px bg-line-1" />
          {facets.kinds.map((k) => (
            <Chip key={k.value} on={kind === k.value} onClick={() => setKind(kind === k.value ? null : k.value)}>
              {KIND_LABEL[k.value] ?? k.value} <span className="mono text-fg-4">{k.count}</span>
            </Chip>
          ))}
          {facets.platforms.length > 1 && <span className="mx-1 h-4 w-px bg-line-1" />}
          {facets.platforms.length > 1 &&
            facets.platforms.map((p) => (
              <Chip key={p.value} on={platform === p.value} onClick={() => setPlatform(platform === p.value ? null : p.value)}>
                {PLATFORM_LABEL[p.value] ?? p.value} <span className="mono text-fg-4">{p.count}</span>
              </Chip>
            ))}
        </div>
      )}

      <div className="t-micro grid grid-cols-[minmax(0,1fr)_96px_88px_80px_96px_120px] gap-3 border-b border-line-1 pb-2">
        <span>Title</span><span>Format</span><span>Size</span><span>Length</span><span>Saved</span><span />
      </div>

      {items && items.length === 0 ? (
        <div className="mt-4 rounded-md border border-dashed border-line-2 p-5">
          <p className="text-fg-2">{filtered ? "No matches." : "No downloads yet."}</p>
          <p className="mt-0.5 text-xs text-fg-3">{filtered ? "Try another word, or clear the filters." : "Finished downloads appear here with their files."}</p>
          {filtered && (
            <button className="btn btn-secondary mt-3" onClick={() => { setText(""); setPlatform(null); setKind(null); }}>Clear filters</button>
          )}
        </div>
      ) : (
        <div ref={parent} className="scroll min-h-0 flex-1">
          <div style={{ height: v.getTotalSize(), position: "relative" }}>
            {v.getVirtualItems().map((row) => {
              const it = items![row.index];
              return (
                <div
                  key={it.id}
                  className="group absolute inset-x-0 grid grid-cols-[minmax(0,1fr)_96px_88px_80px_96px_120px] items-center gap-3 border-b border-line-1"
                  style={{ height: ROW, transform: `translateY(${row.start}px)` }}
                >
                  <div className="flex min-w-0 items-center gap-3">
                    <span className="chip chip-sm">{MONOGRAM[it.platform] ?? "··"}</span>
                    <div className="min-w-0">
                      <div className={`truncate ${it.fileExists ? "" : "text-fg-3 line-through"}`} title={it.title}>{it.title}</div>
                      <div className="t-data truncate text-fg-3">{it.uploader ?? "—"}{it.fileExists ? "" : " · file missing"}</div>
                    </div>
                  </div>
                  <span className="t-data text-fg-2">{it.container} · {it.optionLabel}</span>
                  <span className="t-data text-fg-2">{bytes(it.fileSize)}</span>
                  <span className="t-data text-fg-2">{clock(it.duration)}</span>
                  <span className="t-data text-fg-3">{relativeDate(it.completedAt)}</span>
                  <div className="flex justify-end gap-0.5 opacity-60 group-hover:opacity-100 group-focus-within:opacity-100">
                    <button className="btn btn-icon" title="Open" aria-label="Open" disabled={!it.fileExists} onClick={() => run(() => api.libraryOpen(it.id, false))}><Icon name="play" /></button>
                    <button className="btn btn-icon" title="Show in folder" aria-label="Show in folder" disabled={!it.fileExists} onClick={() => run(() => api.libraryOpen(it.id, true))}><Icon name="folder" /></button>
                    <button className="btn btn-icon" title="Copy source URL" aria-label="Copy source URL" onClick={() => run(() => writeText(it.sourceUrl), "Source URL copied")}><Icon name="copy" /></button>
                    <button className="btn btn-icon" title="Remove…" aria-label="Remove" onClick={() => setConfirm(it)}><Icon name="trash" /></button>
                  </div>
                </div>
              );
            })}
          </div>
        </div>
      )}

      {confirm && (
        <div className="fixed inset-0 z-40 grid place-items-center bg-[rgb(0_0_0/0.5)]" onClick={() => setConfirm(null)} onKeyDown={(e) => e.key === "Escape" && setConfirm(null)}>
          <div role="dialog" aria-modal="true" aria-labelledby="rm-title" className="rise w-[440px] rounded-lg border border-line-2 bg-bg-2 p-5 shadow-[var(--shadow-3)]" onClick={(e) => e.stopPropagation()}>
            <h2 id="rm-title" className="text-lg font-semibold">Remove from library?</h2>
            <p className="mt-1 text-fg-2">
              <span className="selectable">“{confirm.title}”</span> will be removed from the list. You can also delete the file from disk — that can't be undone.
            </p>
            <div className="mt-5 flex justify-end gap-2">
              <button className="btn btn-ghost" autoFocus onClick={() => setConfirm(null)}>Cancel</button>
              <button className="btn btn-secondary" onClick={() => remove(confirm, false)}>Remove from list</button>
              <button className="btn btn-danger" disabled={!confirm.fileExists} onClick={() => remove(confirm, true)}>Delete file</button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
