import { useEffect, useState } from "react";
import { Icon } from "../../icons/Icon";
import { api, toError } from "../../ipc/api";
import type { Settings, ToolStatus } from "../../ipc/types";
import { useApp } from "../../state/store";
import { Row, Stepper, Switch } from "./controls";
import { SignedInSites } from "./SignedInSites";
import { YtDlpUpdater } from "./YtDlpUpdater";

export default function SettingsView() {
  const settings = useApp((s) => s.settings);
  const { setSettings, toast } = useApp.getState();
  const [tools, setTools] = useState<ToolStatus | null>(null);

  useEffect(() => {
    api.toolStatus().then(setTools, () => {});
  }, []);

  const save = (patch: Partial<Settings>) => {
    if (!settings) return;
    api.setSettings({ ...settings, ...patch }).then(setSettings, (e) => toast({ tone: "err", title: "Couldn't save settings", body: toError(e).message }));
  };

  const pick = () => api.chooseDownloadDir().then((s) => s && setSettings(s), (e) => toast({ tone: "err", title: "Couldn't change the folder", body: toError(e).message }));

  if (!settings) return null;
  const tool = (name: string, path: string | null | undefined) => (
    <div className="t-data flex min-w-0 gap-3">
      <span className="w-20 shrink-0 text-fg-3">{name}</span>
      {path ? <span className="selectable min-w-0 truncate text-fg-2" title={path}>{path}</span> : <span className="text-err">not found</span>}
    </div>
  );

  return (
    <div className="scroll rise h-full px-6 pb-8">
      <h1 className="pt-5 pb-4 text-lg font-semibold tracking-[-0.01em]">Settings</h1>

      <div className="t-micro mb-1">Downloads</div>
      <Row label="Download folder" hint="New files are saved here. Existing files are never overwritten.">
        <span className="t-data min-w-0 flex-1 truncate text-fg-2" title={settings.downloadDir}>{settings.downloadDir}</span>
        <button className="btn btn-secondary" onClick={pick}><Icon name="folder" />Change</button>
      </Row>
      <Row label="Simultaneous downloads" hint="How many items transfer at the same time (1–8).">
        <Stepper label="simultaneous downloads" value={settings.maxConcurrent} min={1} max={8} onChange={(n) => save({ maxConcurrent: n })} />
      </Row>
      <Row label="Connections per file" hint="Byte-range segments for servers that support them (1–16). More isn't always faster.">
        <Stepper label="connections per file" value={settings.maxSegments} min={1} max={16} onChange={(n) => save({ maxSegments: n })} />
      </Row>

      <div className="t-micro mt-8 mb-1">Behaviour</div>
      <Row label="Notifications" hint="A desktop notification when a download finishes or fails while Rabbit isn't in front.">
        <Switch label="Notifications" on={settings.notifications} onChange={(v) => save({ notifications: v })} />
      </Row>

      <div className="t-micro mt-8 mb-1">Sessions</div>
      <SignedInSites settings={settings} save={save} />

      <div className="t-micro mt-8 mb-1">Tools</div>
      <YtDlpUpdater onChanged={() => api.toolStatus().then(setTools, () => {})} />
      <div className="grid min-w-0 gap-1 border-t border-line-1 py-3.5">
        {tool("yt-dlp", tools?.tools.ytdlp)}
        {tool("FFmpeg", tools?.tools.ffmpeg)}
        {tool("ffprobe", tools?.tools.ffprobe)}
        {tool("JS runtime", tools?.tools.jsRuntime)}
      </div>

      <div className="t-micro mt-8 mb-1">About</div>
      <div className="border-t border-line-1 py-3.5 text-fg-2">
        Rabbit <span className="mono">{tools?.version ?? ""}</span> · local-first: no account, no telemetry. Downloads go directly from the source to this PC.
      </div>
    </div>
  );
}
