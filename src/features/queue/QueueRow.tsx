import { memo, useState } from "react";
import { Icon } from "../../icons/Icon";
import { api, toError } from "../../ipc/api";
import type { TaskInfo } from "../../ipc/types";
import { Ruler } from "../../design/Ruler";
import { bytes, clock, MONOGRAM, progressPair, speed } from "../../lib/format";
import { isActive, useApp } from "../../state/store";

const STATE_LABEL: Record<TaskInfo["state"], string> = {
  queued: "Queued",
  downloading: "",
  processing: "Merging",
  verifying: "Verifying",
  paused: "Paused",
  completed: "Verified",
  failed: "Failed",
  cancelled: "Cancelled",
};

function act(fn: () => Promise<unknown>) {
  fn().catch((e) => useApp.getState().toast({ tone: "err", title: "Action failed", body: toError(e).message }));
}

export const QueueRow = memo(function QueueRow({ id }: { id: string }) {
  const task = useApp((s) => s.tasks[id]);
  const progress = useApp((s) => s.progress[id]);
  const [showDetail, setShowDetail] = useState(false);
  if (!task) return null;

  const done = progress?.done ?? task.done;
  const total = progress?.total ?? task.total;
  const running = task.state === "downloading";

  let middle: React.ReactNode;
  if (running) {
    middle = (
      <>
        <span>{progressPair(done, total)}</span>
        <span className="text-fg-1">{speed(progress?.speed ?? 0)}</span>
        <span>{clock(progress?.eta)}</span>
      </>
    );
  } else if (task.state === "completed") {
    middle = (
      <>
        <span>{bytes(total ?? done)}</span>
        <span className="flex items-center gap-1.5 font-sans text-ok"><Icon name="check" />Verified</span>
        <span />
      </>
    );
  } else if (task.state === "failed") {
    middle = (
      <>
        <span>{done > 0 ? progressPair(done, total) : ""}</span>
        <span className="flex items-center gap-1.5 font-sans text-err"><Icon name="alert" />Failed</span>
        <span />
      </>
    );
  } else if (task.state === "paused") {
    middle = (
      <>
        <span>{total ? progressPair(done, total) : done > 0 ? bytes(done) : ""}</span>
        <span className="flex items-center gap-1.5 font-sans text-fg-2" title={task.interrupted ? "Rabbit closed while this was downloading. Saved data is kept." : undefined}>
          <Icon name="pause" />{task.interrupted ? "Interrupted" : "Paused"}
        </span>
        <span />
      </>
    );
  } else if (task.state === "cancelled") {
    middle = (
      <>
        <span className="font-sans text-fg-3">Partial data discarded</span>
        <span className="font-sans text-fg-2">Cancelled</span>
        <span />
      </>
    );
  } else {
    middle = (
      <>
        <span>{total ? progressPair(done, total) : ""}</span>
        <span className="font-sans text-fg-2">{STATE_LABEL[task.state]}</span>
        <span />
      </>
    );
  }

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.target !== e.currentTarget) return; // keys inside buttons keep their own meaning
    if (e.key === " ") {
      e.preventDefault();
      if (isActive(task)) act(() => api.pauseTask(id));
      else if (task.state === "paused") act(() => api.resumeTask(id));
    } else if (e.key === "Delete") {
      if (isActive(task) || task.state === "paused") act(() => api.cancelTask(id));
      else act(() => api.removeTask(id));
    } else if (e.key === "Enter" && task.state === "completed") {
      act(() => api.openTask(id, e.shiftKey));
    }
  };

  return (
    <div
      className="group rounded-[3px] border-t border-line-1 first:border-t-0 focus-visible:outline-offset-[-2px]"
      tabIndex={0}
      onKeyDown={onKeyDown}
      aria-label={`${task.title}, ${task.state === "paused" && task.interrupted ? "interrupted" : task.state}`}
    >
      <div className="grid h-14 grid-cols-[64px_minmax(0,1fr)_minmax(240px,340px)_auto] items-center gap-4">
        <div className="h-9 w-16 overflow-hidden rounded-[2px] border border-line-1 bg-bg-0">
          {task.thumbnail && <img src={task.thumbnail} alt="" className="h-full w-full object-cover" draggable={false} />}
        </div>
        <div className="min-w-0">
          <div className="truncate font-medium" title={task.title}>{task.title}</div>
          <div className="t-data mt-0.5 flex items-center gap-2 text-fg-3">
            <span className="chip chip-sm">{MONOGRAM[task.platform]}</span>
            <span className="truncate">{task.container} · {task.optionLabel}{task.uploader ? ` · ${task.uploader}` : ""}</span>
          </div>
        </div>
        <div>
          <div className="t-data mb-1.5 flex justify-between text-fg-2">{middle}</div>
          <Ruler done={done} total={total} state={task.state} segments={progress?.segments} />
        </div>
        <div className="flex justify-end gap-0.5 opacity-70 group-hover:opacity-100 group-focus-within:opacity-100">
          {isActive(task) && (
            <button className="btn btn-icon" title="Pause (Space)" aria-label="Pause" onClick={() => act(() => api.pauseTask(id))}>
              <Icon name="pause" />
            </button>
          )}
          {task.state === "paused" && (
            <button className="btn btn-icon text-amber" title="Resume (Space)" aria-label="Resume" onClick={() => act(() => api.resumeTask(id))}>
              <Icon name="play" />
            </button>
          )}
          {(isActive(task) || task.state === "paused") && (
            <button className="btn btn-icon" title="Cancel and discard partial data" aria-label="Cancel" onClick={() => act(() => api.cancelTask(id))}>
              <Icon name="x" />
            </button>
          )}
          {(task.state === "failed" || task.state === "cancelled") && (
            <button className="btn btn-icon" title="Retry" aria-label="Retry" onClick={() => act(() => api.resumeTask(id))}>
              <Icon name="retry" />
            </button>
          )}
          {task.state === "completed" && (
            <>
              <button className="btn btn-icon" title="Open" aria-label="Open file" onClick={() => act(() => api.openTask(id, false))}>
                <Icon name="play" />
              </button>
              <button className="btn btn-icon" title="Show in folder" aria-label="Show in folder" onClick={() => act(() => api.openTask(id, true))}>
                <Icon name="folder" />
              </button>
            </>
          )}
          {!isActive(task) && task.state !== "paused" && (
            <button className="btn btn-icon" title="Remove from list" aria-label="Remove from list" onClick={() => act(() => api.removeTask(id))}>
              <Icon name="trash" />
            </button>
          )}
        </div>
      </div>
      {task.state === "failed" && task.error && (
        <div className="-mt-1 mb-2.5 ml-20 text-xs leading-4">
          <span className="text-err">{task.error.message}</span>
          {task.error.detail && (
            <button className="ml-2 text-fg-3 underline-offset-2 hover:text-fg-1 hover:underline" onClick={() => setShowDetail((v) => !v)}>
              {showDetail ? "Hide details" : "Details"}
            </button>
          )}
          {showDetail && task.error.detail && <pre className="selectable mono mt-1 whitespace-pre-wrap text-fg-3">{task.error.detail}</pre>}
        </div>
      )}
    </div>
  );
});
