import React, { useEffect, useState } from "react";
import {
  FileCode2,
  HardDrive,
  Hexagon,
  Loader2,
  Minus,
  Network,
  Plus,
  Rocket,
  RotateCcw,
  Save,
  Server,
  Trash2,
  X,
} from "lucide-react";
import { Button } from "../ui/Button";
import { Switch } from "../ui/Switch";
import { useModal } from "../ui/ModalContext";
import { useToast } from "../ui/Toast";
import { api } from "../../lib/api";
import {
  CONTROL_PLANE_OPTIONS,
  NEW_MOUNT,
  NEW_PORT_MAPPING,
  PORT_RANGE_HINT,
  RAM_PER_NODE_GB,
  applyPreset,
  maxWorkers,
  presetFromSpec,
  sameSpec,
  specFromDefaults,
  totalNodes,
  validateClusterName,
  validateContainerPort,
  validateCidr,
  validateFeatureGate,
  validateHostPort,
  validateMount,
  validatePresetName,
  validateSpec,
} from "../../lib/clusterSpec";
import type {
  ClusterConfigPreview,
  ClusterPreset,
  ClusterSpec,
  HostMountSpec,
  KindProgressPayload,
  NodeImageOption,
  PortMappingSpec,
} from "../../types";

const STEPS = [
  { label: "Basics", icon: Hexagon },
  { label: "Topology", icon: Server },
  { label: "Networking", icon: Network },
  { label: "Storage & features", icon: HardDrive },
  { label: "Review", icon: FileCode2 },
] as const;
const LAST_STEP = STEPS.length - 1;

/** Select value that reveals the free-text image field. */
const CUSTOM_IMAGE = "__custom__";

const INPUT =
  "h-8 w-full rounded-none border border-zinc-800 bg-zinc-950 px-2 text-xs text-zinc-200 placeholder:text-zinc-600 focus:outline-none focus:border-blue-500 disabled:opacity-50";

/** Number input value: an empty field becomes NaN so the validators flag it. */
const parseNum = (s: string): number => (s.trim() === "" ? NaN : Number(s));
const numText = (n: number): string => (Number.isNaN(n) ? "" : String(n));

const segmentClass = (on: boolean): string =>
  `h-8 w-12 text-xs font-medium rounded-none border-r border-zinc-800 last:border-r-0 transition-colors ${
    on ? "bg-blue-600 text-white" : "bg-zinc-950 text-zinc-400 hover:text-zinc-200 hover:bg-zinc-800/60"
  }`;

function Field({ label, hint, children }: { label: string; hint?: string; children: React.ReactNode }) {
  return (
    <label className="block space-y-1.5">
      <span className="block text-[10px] uppercase tracking-wider text-zinc-500">{label}</span>
      {children}
      {hint && <span className="block text-[11px] leading-relaxed text-zinc-500">{hint}</span>}
    </label>
  );
}

/** Inline problem text under a field; renders nothing when there is no problem. */
function Issue({ message }: { message: string | null }) {
  if (!message) return null;
  return <span className="block text-[10px] text-rose-400">{message}</span>;
}

interface CreateClusterWizardProps {
  /** Settings → Kubernetes defaults used to pre-fill the spec. */
  defaults: { nodes: number; image: string };
  /** A create is running; the wizard stays open and shows its output. */
  busy: boolean;
  logLines: KindProgressPayload[];
  existingNames: string[];
  onCreate: (name: string, spec: ClusterSpec, config?: string) => Promise<void>;
  onClose: () => void;
}

/**
 * Multi-step "Create cluster" modal. The settings build a kind config that the
 * backend previews; the Review step can replace that config with hand-edited
 * YAML, which then wins over the settings on create.
 */
export function CreateClusterWizard({
  defaults,
  busy,
  logLines,
  existingNames,
  onCreate,
  onClose,
}: CreateClusterWizardProps) {
  const modal = useModal();
  const toast = useToast();
  const [step, setStep] = useState(0);
  const [name, setName] = useState("");
  const [spec, setSpec] = useState<ClusterSpec>(() => specFromDefaults(defaults.nodes, defaults.image));

  const [presets, setPresets] = useState<ClusterPreset[]>([]);
  const [presetDraft, setPresetDraft] = useState<{ name: string; description: string } | null>(null);

  // null after a failed lookup: the image is then a free-text field only.
  const [nodeImages, setNodeImages] = useState<NodeImageOption[] | null>(null);
  const [imagesLoading, setImagesLoading] = useState(true);
  const [customImage, setCustomImage] = useState(false);

  const [yamlEdited, setYamlEdited] = useState(false);
  const [yamlText, setYamlText] = useState("");
  const [showFinal, setShowFinal] = useState(false);
  const [preview, setPreview] = useState<ClusterConfigPreview | null>(null);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const [previewPending, setPreviewPending] = useState(true);

  useEffect(() => {
    api
      .listClusterPresets()
      .then(setPresets)
      .catch((e) => toast.error("Presets unavailable", String(e)));
    api
      .listKindNodeImages()
      .then((list) => {
        setNodeImages(list);
        setCustomImage(spec.image !== null && !list.some((o) => o.image === spec.image));
      })
      .catch(() => setNodeImages(null))
      .finally(() => setImagesLoading(false));
    // Runs once: the lists are fetched when the wizard opens.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // A hand-edited YAML is sent as `config`; otherwise the backend builds it from the spec.
  const configForPreview = yamlEdited ? yamlText : undefined;

  // Debounced preview of every settings change (and of each YAML edit).
  useEffect(() => {
    let cancelled = false;
    setPreviewPending(true);
    const timer = window.setTimeout(() => {
      api
        .previewClusterConfig(spec, configForPreview)
        .then((res) => {
          if (cancelled) return;
          setPreview(res);
          setPreviewError(null);
          // Only the generated YAML feeds the editor; a hand-edited one is kept as typed.
          if (configForPreview === undefined) setYamlText(res.yaml);
        })
        .catch((e) => {
          if (cancelled) return;
          setPreview(null);
          setPreviewError(String(e));
        })
        .finally(() => {
          if (!cancelled) setPreviewPending(false);
        });
    }, 300);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [spec, configForPreview]);

  // Escape closes, but never while a create is running.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !busy) onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [busy, onClose]);

  const nameIssue = validateClusterName(name);
  const nameTaken = nameIssue === null && existingNames.includes(name.trim());
  const specProblems = yamlEdited ? [] : validateSpec(spec);
  const errors = [
    ...(previewError ? [previewError] : []),
    ...specProblems,
    ...(preview?.errors ?? []),
  ];
  const canCreate =
    !busy && !previewPending && nameIssue === null && !nameTaken && preview !== null && errors.length === 0;

  const patch = (p: Partial<ClusterSpec>) => setSpec((s) => ({ ...s, ...p }));
  const patchPort = (i: number, p: Partial<PortMappingSpec>) =>
    patch({ portMappings: spec.portMappings.map((m, j) => (j === i ? { ...m, ...p } : m)) });
  const patchMount = (i: number, m: Partial<HostMountSpec>) =>
    patch({ mounts: spec.mounts.map((x, j) => (j === i ? { ...x, ...m } : x)) });
  const patchGate = (i: number, value: string) =>
    patch({ featureGates: spec.featureGates.map((g, j) => (j === i ? value : g)) });

  const goto = (i: number) => setStep(Math.max(0, Math.min(LAST_STEP, i)));

  const submit = () => {
    if (canCreate) void onCreate(name.trim(), spec, configForPreview);
  };

  // Enter advances; on the last step it creates. Inside the YAML editor it is a newline.
  const onFormSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (step < LAST_STEP) goto(step + 1);
    else submit();
  };

  const savePreset = async () => {
    if (!presetDraft) return;
    const issue = validatePresetName(presetDraft.name);
    if (issue) {
      toast.error("Preset not saved", issue);
      return;
    }
    try {
      setPresets(await api.saveClusterPreset(presetFromSpec(presetDraft.name, presetDraft.description, spec)));
      toast.success("Preset saved", presetDraft.name.trim());
      setPresetDraft(null);
    } catch (e) {
      toast.error("Preset not saved", String(e));
    }
  };

  const removePreset = async (preset: ClusterPreset) => {
    const ok = await modal.showConfirm(
      `Delete preset '${preset.name}'? Clusters already created from it are not affected.`,
      "Delete Preset",
      "danger"
    );
    if (!ok) return;
    try {
      setPresets(await api.deleteClusterPreset(preset.name));
    } catch (e) {
      toast.error("Delete failed", String(e));
    }
  };

  const nodeTotal = totalNodes(spec);
  const workerCap = maxWorkers(spec.controlPlanes);
  const logTail = logLines.slice(-12);

  const renderBasics = () => (
    <div className="space-y-6">
      <Field label="Cluster name">
        <input
          type="text"
          autoFocus
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="my-cluster"
          disabled={busy}
          className={`${INPUT} h-9`}
        />
        <Issue message={name.trim() ? nameIssue : null} />
        <Issue message={nameTaken ? "A cluster with this name already exists." : null} />
      </Field>

      <div className="space-y-2">
        <div className="flex items-center justify-between">
          <span className="text-[10px] uppercase tracking-wider text-zinc-500">Preset</span>
          {presetDraft === null && (
            <Button
              variant="ghost"
              size="sm"
              className="h-7 rounded-none px-2 text-[11px]"
              onClick={() => setPresetDraft({ name: "", description: "" })}
              disabled={busy}
            >
              <Save className="h-3.5 w-3.5" />
              Save current as preset
            </Button>
          )}
        </div>
        <div className="grid gap-2 sm:grid-cols-2">
          {presets.map((p) => {
            const selected = sameSpec(spec, applyPreset(p));
            return (
              <div
                key={p.name}
                className={`flex items-start gap-2 border px-3 py-2 ${
                  selected ? "border-blue-500/60 bg-blue-500/10" : "border-zinc-800 bg-zinc-950 hover:border-zinc-700"
                }`}
              >
                <button
                  type="button"
                  disabled={busy}
                  aria-pressed={selected}
                  onClick={() => setSpec(applyPreset(p))}
                  className="min-w-0 flex-1 text-left"
                >
                  <span className="flex items-center gap-2 text-xs font-medium text-white">
                    {p.name}
                    {p.builtin && (
                      <span className="border border-zinc-700 px-1 text-[9px] uppercase tracking-wider text-zinc-500">
                        built-in
                      </span>
                    )}
                  </span>
                  <span className="mt-0.5 block text-[11px] leading-relaxed text-zinc-500">{p.description}</span>
                </button>
                {!p.builtin && (
                  <Button
                    variant="ghost"
                    size="icon"
                    title={`Delete preset ${p.name}`}
                    disabled={busy}
                    onClick={() => removePreset(p)}
                    className="h-6 w-6 rounded-none text-zinc-500 hover:text-rose-400"
                  >
                    <Trash2 className="h-3.5 w-3.5" />
                  </Button>
                )}
              </div>
            );
          })}
        </div>
        <p className="text-[11px] text-zinc-500">Applying a preset replaces the settings on every step.</p>
        {presetDraft && (
          <div className="space-y-2 border border-zinc-800 bg-zinc-950 p-3">
            <div className="grid gap-2 sm:grid-cols-[minmax(0,1fr)_minmax(0,2fr)]">
              <input
                type="text"
                aria-label="Preset name"
                placeholder="Preset name"
                value={presetDraft.name}
                onChange={(e) => setPresetDraft({ ...presetDraft, name: e.target.value })}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault();
                    void savePreset();
                  }
                }}
                className={INPUT}
              />
              <input
                type="text"
                aria-label="Preset description"
                placeholder="Description (optional)"
                value={presetDraft.description}
                onChange={(e) => setPresetDraft({ ...presetDraft, description: e.target.value })}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault();
                    void savePreset();
                  }
                }}
                className={INPUT}
              />
            </div>
            <div className="flex justify-end gap-2">
              <Button variant="ghost" size="sm" className="rounded-none" onClick={() => setPresetDraft(null)}>
                Cancel
              </Button>
              <Button variant="primary" size="sm" className="rounded-none" onClick={savePreset}>
                <Save className="h-3.5 w-3.5" />
                Save preset
              </Button>
            </div>
          </div>
        )}
      </div>

      <Field
        label="Kubernetes version"
        hint="The node image sets the Kubernetes version. Local images are already on this machine."
      >
        {imagesLoading ? (
          <select disabled className={INPUT}>
            <option>Loading versions…</option>
          </select>
        ) : nodeImages === null ? (
          <input
            type="text"
            value={spec.image ?? ""}
            onChange={(e) => patch({ image: e.target.value.trim() || null })}
            placeholder="kind's bundled default"
            disabled={busy}
            className={INPUT}
          />
        ) : (
          <>
            <select
              value={customImage ? CUSTOM_IMAGE : (spec.image ?? "")}
              disabled={busy}
              onChange={(e) => {
                if (e.target.value === CUSTOM_IMAGE) {
                  setCustomImage(true);
                  return;
                }
                setCustomImage(false);
                patch({ image: e.target.value || null });
              }}
              className={INPUT}
            >
              <option value="">kind's bundled default</option>
              {nodeImages.map((o) => (
                <option key={o.image} value={o.image}>
                  {o.version}
                  {o.default ? " · default" : ""}
                  {o.local ? " · local" : ""}
                </option>
              ))}
              <option value={CUSTOM_IMAGE}>Custom image…</option>
            </select>
            {customImage && (
              <input
                type="text"
                aria-label="Custom node image"
                value={spec.image ?? ""}
                onChange={(e) => patch({ image: e.target.value.trim() || null })}
                placeholder="kindest/node:v1.31.0"
                disabled={busy}
                className={`${INPUT} mt-2 font-mono`}
              />
            )}
          </>
        )}
      </Field>
    </div>
  );

  const renderTopology = () => (
    <div className="space-y-6">
      <Field label="Control planes" hint="3 or 5 give a highly available API server (etcd quorum). 1 is the simplest.">
        <div role="radiogroup" aria-label="Control planes" className="inline-flex border border-zinc-800">
          {CONTROL_PLANE_OPTIONS.map((n) => (
            <button
              key={n}
              type="button"
              role="radio"
              aria-checked={spec.controlPlanes === n}
              disabled={busy}
              onClick={() => patch({ controlPlanes: n, workers: Math.min(spec.workers, maxWorkers(n)) })}
              className={segmentClass(spec.controlPlanes === n)}
            >
              {n}
            </button>
          ))}
        </div>
      </Field>

      <Field label="Workers" hint={`0–${workerCap} with ${spec.controlPlanes} control plane${spec.controlPlanes === 1 ? "" : "s"}.`}>
        <div className="inline-flex items-center border border-zinc-800">
          <Button
            variant="ghost"
            size="icon"
            title="Remove a worker"
            aria-label="Remove a worker"
            disabled={busy || spec.workers <= 0}
            onClick={() => patch({ workers: spec.workers - 1 })}
            className="h-8 w-8 rounded-none"
          >
            <Minus className="h-3.5 w-3.5" />
          </Button>
          <span className="w-12 text-center font-mono text-sm tabular-nums text-white">{spec.workers}</span>
          <Button
            variant="ghost"
            size="icon"
            title="Add a worker"
            aria-label="Add a worker"
            disabled={busy || spec.workers >= workerCap}
            onClick={() => patch({ workers: spec.workers + 1 })}
            className="h-8 w-8 rounded-none"
          >
            <Plus className="h-3.5 w-3.5" />
          </Button>
        </div>
      </Field>

      <div className="grid gap-3 sm:grid-cols-2">
        <div className="border border-zinc-800 bg-zinc-950 px-4 py-3">
          <p className="text-[10px] uppercase tracking-wider text-zinc-500">Total nodes</p>
          <p className="mt-1 font-mono text-xl tabular-nums text-white">{nodeTotal}</p>
          <p className="text-[11px] text-zinc-500">
            {spec.controlPlanes} control plane · {spec.workers} worker
          </p>
        </div>
        <div className="border border-zinc-800 bg-zinc-950 px-4 py-3">
          <p className="text-[10px] uppercase tracking-wider text-zinc-500">RAM hint</p>
          <p className="mt-1 font-mono text-xl tabular-nums text-white">
            ~{(nodeTotal * RAM_PER_NODE_GB).toFixed(1)} GB
          </p>
          <p className="text-[11px] text-zinc-500">
            Rough estimate at {RAM_PER_NODE_GB} GB per node, before your workloads.
          </p>
        </div>
      </div>
    </div>
  );

  const renderNetworking = () => (
    <div className="space-y-6">
      <div className="space-y-3 border border-zinc-800 bg-zinc-950 p-3">
        <Switch
          checked={spec.ingress}
          disabled={busy}
          onChange={(next) => patch({ ingress: next })}
          label="Ingress ports"
          title="Forward host ports to the ingress controller (HTTP and HTTPS)"
        />
        {spec.ingress && (
          <div className="grid gap-3 sm:grid-cols-2">
            <Field label="Host HTTP port">
              <input
                type="number"
                min={1}
                max={65535}
                value={numText(spec.ingressHttpPort)}
                disabled={busy}
                onChange={(e) => patch({ ingressHttpPort: parseNum(e.target.value) })}
                className={INPUT}
              />
              <Issue message={validateHostPort(spec.ingressHttpPort)} />
            </Field>
            <Field label="Host HTTPS port">
              <input
                type="number"
                min={1}
                max={65535}
                value={numText(spec.ingressHttpsPort)}
                disabled={busy}
                onChange={(e) => patch({ ingressHttpsPort: parseNum(e.target.value) })}
                className={INPUT}
              />
              <Issue message={validateHostPort(spec.ingressHttpsPort)} />
            </Field>
          </div>
        )}
      </div>

      <div className="space-y-2">
        <div className="flex items-center justify-between">
          <span className="text-[10px] uppercase tracking-wider text-zinc-500">Port mappings</span>
          <Button
            variant="ghost"
            size="sm"
            className="h-7 rounded-none px-2 text-[11px]"
            disabled={busy}
            onClick={() => patch({ portMappings: [...spec.portMappings, { ...NEW_PORT_MAPPING }] })}
          >
            <Plus className="h-3.5 w-3.5" />
            Add mapping
          </Button>
        </div>
        {spec.portMappings.length > 0 && (
          <table className="w-full text-xs">
            <thead>
              <tr className="text-left text-[10px] uppercase tracking-wider text-zinc-500">
                <th className="pb-1.5 font-medium">Container port</th>
                <th className="pb-1.5 font-medium">Host port</th>
                <th className="pb-1.5 font-medium">Protocol</th>
                <th className="pb-1.5" />
              </tr>
            </thead>
            <tbody>
              {spec.portMappings.map((m, i) => (
                <tr key={i} className="align-top">
                  <td className="pr-2 pb-2">
                    <input
                      type="number"
                      min={1}
                      max={65535}
                      aria-label={`Mapping ${i + 1} container port`}
                      value={numText(m.containerPort)}
                      disabled={busy}
                      onChange={(e) => patchPort(i, { containerPort: parseNum(e.target.value) })}
                      className={`${INPUT} w-28`}
                    />
                    <Issue message={validateContainerPort(m.containerPort)} />
                  </td>
                  <td className="pr-2 pb-2">
                    <input
                      type="number"
                      min={1}
                      max={65535}
                      aria-label={`Mapping ${i + 1} host port`}
                      value={numText(m.hostPort)}
                      disabled={busy}
                      onChange={(e) => patchPort(i, { hostPort: parseNum(e.target.value) })}
                      className={`${INPUT} w-28`}
                    />
                    <Issue message={validateHostPort(m.hostPort)} />
                  </td>
                  <td className="pr-2 pb-2">
                    <select
                      aria-label={`Mapping ${i + 1} protocol`}
                      value={m.protocol}
                      disabled={busy}
                      onChange={(e) => patchPort(i, { protocol: e.target.value as PortMappingSpec["protocol"] })}
                      className={`${INPUT} w-24`}
                    >
                      <option value="TCP">TCP</option>
                      <option value="UDP">UDP</option>
                      <option value="SCTP">SCTP</option>
                    </select>
                  </td>
                  <td className="pb-2 text-right">
                    <Button
                      variant="ghost"
                      size="icon"
                      title="Remove mapping"
                      aria-label={`Remove mapping ${i + 1}`}
                      disabled={busy}
                      onClick={() => patch({ portMappings: spec.portMappings.filter((_, j) => j !== i) })}
                      className="h-7 w-7 rounded-none text-zinc-500 hover:text-rose-400"
                    >
                      <X className="h-3.5 w-3.5" />
                    </Button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
        <p className="text-[11px] leading-relaxed text-zinc-500">{PORT_RANGE_HINT}</p>
      </div>

      <div className="grid gap-3 sm:grid-cols-2">
        <Field label="Pod subnet" hint="Empty keeps kind's default.">
          <input
            type="text"
            value={spec.podSubnet ?? ""}
            placeholder="10.244.0.0/16"
            disabled={busy}
            onChange={(e) => patch({ podSubnet: e.target.value.trim() || null })}
            className={`${INPUT} font-mono`}
          />
          <Issue message={validateCidr(spec.podSubnet)} />
        </Field>
        <Field label="Service subnet" hint="Empty keeps kind's default.">
          <input
            type="text"
            value={spec.serviceSubnet ?? ""}
            placeholder="10.96.0.0/16"
            disabled={busy}
            onChange={(e) => patch({ serviceSubnet: e.target.value.trim() || null })}
            className={`${INPUT} font-mono`}
          />
          <Issue message={validateCidr(spec.serviceSubnet)} />
        </Field>
      </div>
    </div>
  );

  const renderStorage = () => (
    <div className="space-y-6">
      <div className="space-y-2">
        <div className="flex items-center justify-between">
          <span className="text-[10px] uppercase tracking-wider text-zinc-500">Host mounts</span>
          <Button
            variant="ghost"
            size="sm"
            className="h-7 rounded-none px-2 text-[11px]"
            disabled={busy}
            onClick={() => patch({ mounts: [...spec.mounts, { ...NEW_MOUNT }] })}
          >
            <Plus className="h-3.5 w-3.5" />
            Add mount
          </Button>
        </div>
        {spec.mounts.length > 0 && (
          <table className="w-full text-xs">
            <thead>
              <tr className="text-left text-[10px] uppercase tracking-wider text-zinc-500">
                <th className="pb-1.5 font-medium">Host path</th>
                <th className="pb-1.5 font-medium">Container path</th>
                <th className="pb-1.5 font-medium">Read-only</th>
                <th className="pb-1.5" />
              </tr>
            </thead>
            <tbody>
              {spec.mounts.map((m, i) => {
                const issue = m.hostPath || m.containerPath ? validateMount(m) : null;
                return (
                  <tr key={i} className="align-top">
                    <td className="pr-2 pb-2">
                      <input
                        type="text"
                        aria-label={`Mount ${i + 1} host path`}
                        value={m.hostPath}
                        disabled={busy}
                        placeholder="/home/me/data"
                        onChange={(e) => patchMount(i, { hostPath: e.target.value })}
                        className={`${INPUT} font-mono`}
                      />
                      <Issue message={issue} />
                    </td>
                    <td className="pr-2 pb-2">
                      <input
                        type="text"
                        aria-label={`Mount ${i + 1} container path`}
                        value={m.containerPath}
                        disabled={busy}
                        placeholder="/data"
                        onChange={(e) => patchMount(i, { containerPath: e.target.value })}
                        className={`${INPUT} font-mono`}
                      />
                    </td>
                    <td className="pr-2 pb-2 pt-1.5">
                      <input
                        type="checkbox"
                        aria-label={`Mount ${i + 1} read-only`}
                        checked={m.readOnly}
                        disabled={busy}
                        onChange={(e) => patchMount(i, { readOnly: e.target.checked })}
                        className="h-3.5 w-3.5 accent-blue-500"
                      />
                    </td>
                    <td className="pb-2 text-right">
                      <Button
                        variant="ghost"
                        size="icon"
                        title="Remove mount"
                        aria-label={`Remove mount ${i + 1}`}
                        disabled={busy}
                        onClick={() => patch({ mounts: spec.mounts.filter((_, j) => j !== i) })}
                        className="h-7 w-7 rounded-none text-zinc-500 hover:text-rose-400"
                      >
                        <X className="h-3.5 w-3.5" />
                      </Button>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
        <p className="text-[11px] text-zinc-500">Host folders are bind-mounted into every node.</p>
      </div>

      <div className="space-y-2">
        <div className="flex items-center justify-between">
          <span className="text-[10px] uppercase tracking-wider text-zinc-500">Feature gates</span>
          <Button
            variant="ghost"
            size="sm"
            className="h-7 rounded-none px-2 text-[11px]"
            disabled={busy}
            onClick={() => patch({ featureGates: [...spec.featureGates, ""] })}
          >
            <Plus className="h-3.5 w-3.5" />
            Add gate
          </Button>
        </div>
        {spec.featureGates.map((g, i) => (
          <div key={i} className="flex items-start gap-2">
            <div className="min-w-0 flex-1">
              <input
                type="text"
                aria-label={`Feature gate ${i + 1}`}
                value={g}
                placeholder="EphemeralContainers=true"
                disabled={busy}
                onChange={(e) => patchGate(i, e.target.value)}
                className={`${INPUT} font-mono`}
              />
              <Issue message={g.trim() ? validateFeatureGate(g) : null} />
            </div>
            <Button
              variant="ghost"
              size="icon"
              title="Remove feature gate"
              aria-label={`Remove feature gate ${i + 1}`}
              disabled={busy}
              onClick={() => patch({ featureGates: spec.featureGates.filter((_, j) => j !== i) })}
              className="h-8 w-8 rounded-none text-zinc-500 hover:text-rose-400"
            >
              <X className="h-3.5 w-3.5" />
            </Button>
          </div>
        ))}
      </div>

      <div className="border border-zinc-800 bg-zinc-950 p-3">
        <Switch
          checked={spec.localRegistry}
          disabled={busy}
          onChange={(next) => patch({ localRegistry: next })}
          label="Connect to the local registry"
          title="Start the shared registry on localhost:5001 if needed and connect this cluster to it"
        />
        <p className="mt-2 text-[11px] text-zinc-500">
          Push images to localhost:5001 from the Images page and pull them in this cluster without loading them.
        </p>
      </div>
    </div>
  );

  const renderReview = () => (
    <div className="space-y-4">
      {yamlEdited && (
        <div className="border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-[11px] text-amber-300">
          The YAML was edited by hand and is used as typed; the settings on the other steps are ignored until you
          reset it.
        </div>
      )}

      {previewPending && (
        <p className="flex items-center gap-2 text-xs text-zinc-500">
          <Loader2 className="h-3.5 w-3.5 animate-spin" />
          Checking the configuration…
        </p>
      )}

      {errors.length > 0 && (
        <ul role="alert" className="space-y-0.5 border border-rose-500/30 bg-rose-500/10 px-3 py-2 text-[11px] text-rose-300">
          {errors.map((msg, i) => (
            <li key={i}>• {msg}</li>
          ))}
        </ul>
      )}
      {preview && preview.warnings.length > 0 && (
        <ul className="space-y-0.5 border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-[11px] text-amber-300">
          {preview.warnings.map((msg, i) => (
            <li key={i}>• {msg}</li>
          ))}
        </ul>
      )}

      <div className="flex flex-wrap items-center justify-between gap-2">
        <span className="text-[10px] uppercase tracking-wider text-zinc-500">
          {showFinal ? "Final config (read-only, as kind will run it)" : "kind config"}
        </span>
        <div className="flex items-center gap-2">
          <Button
            variant="ghost"
            size="sm"
            className="h-7 rounded-none px-2 text-[11px]"
            disabled={busy || !yamlEdited}
            onClick={() => setYamlEdited(false)}
          >
            <RotateCcw className="h-3.5 w-3.5" />
            Reset to generated
          </Button>
          <Button
            variant="outline"
            size="sm"
            className="h-7 rounded-none px-2 text-[11px]"
            disabled={!preview?.finalYaml}
            onClick={() => setShowFinal((v) => !v)}
          >
            {showFinal ? "Back to editor" : "View final YAML"}
          </Button>
        </div>
      </div>

      {showFinal ? (
        <pre className="max-h-80 overflow-auto border border-zinc-800 bg-zinc-950 p-3 font-mono text-[11px] leading-relaxed text-zinc-300 select-text">
          {preview?.finalYaml}
        </pre>
      ) : (
        <textarea
          value={yamlText}
          onChange={(e) => {
            setYamlEdited(true);
            setYamlText(e.target.value);
          }}
          spellCheck={false}
          disabled={busy}
          aria-label="kind config YAML"
          rows={16}
          className="w-full resize-y border border-zinc-800 bg-zinc-950 p-3 font-mono text-[11px] leading-relaxed text-zinc-200 focus:outline-none focus:border-blue-500 disabled:opacity-60"
        />
      )}
      <p className="text-[11px] text-zinc-500">
        Generated from the settings. Editing it overrides them; the final YAML adds the app's state mounts, API port and
        patches.
      </p>
    </div>
  );

  const bodies = [renderBasics, renderTopology, renderNetworking, renderStorage, renderReview];

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-6 backdrop-blur-sm">
      <div
        className="absolute inset-0"
        onClick={() => !busy && onClose()}
        aria-hidden="true"
      />
      <form
        onSubmit={onFormSubmit}
        role="dialog"
        aria-modal="true"
        aria-label="Create kind cluster"
        className="relative flex h-[min(680px,90vh)] w-[min(960px,96vw)] flex-col overflow-hidden border border-zinc-800 bg-zinc-900 shadow-2xl sm:flex-row"
      >
        <nav aria-label="Steps" className="flex shrink-0 gap-1 overflow-x-auto border-b border-zinc-800 bg-zinc-950 p-2 sm:w-48 sm:flex-col sm:border-b-0 sm:border-r sm:p-3">
          <div className="hidden items-center gap-2 px-2 pb-3 sm:flex">
            <Hexagon className="h-4 w-4 text-blue-400" />
            <span className="text-sm font-semibold text-white">Create cluster</span>
          </div>
          {STEPS.map((s, i) => {
            const Icon = s.icon;
            const current = i === step;
            return (
              <button
                key={s.label}
                type="button"
                aria-current={current ? "step" : undefined}
                disabled={busy}
                onClick={() => goto(i)}
                className={`flex shrink-0 items-center gap-2 px-2.5 py-2 text-left text-xs rounded-none border-l-2 transition-colors disabled:opacity-60 ${
                  current
                    ? "border-blue-500 bg-zinc-800/70 text-white"
                    : "border-transparent text-zinc-400 hover:bg-zinc-800/40 hover:text-zinc-200"
                }`}
              >
                <Icon className="h-3.5 w-3.5 shrink-0" />
                <span className="whitespace-nowrap">{s.label}</span>
              </button>
            );
          })}
        </nav>

        <div className="flex min-h-0 min-w-0 flex-1 flex-col">
          <div className="flex items-center justify-between border-b border-zinc-800 px-5 py-3">
            <div>
              <h3 className="text-sm font-semibold text-white">{STEPS[step].label}</h3>
              <p className="text-[11px] text-zinc-500">
                Step {step + 1} of {STEPS.length} · Enter {step < LAST_STEP ? "continues" : "creates the cluster"} · Esc closes
              </p>
            </div>
            <Button
              variant="ghost"
              size="icon"
              title="Close"
              aria-label="Close"
              disabled={busy}
              onClick={onClose}
              className="h-7 w-7 rounded-none text-zinc-400"
            >
              <X className="h-4 w-4" />
            </Button>
          </div>

          <div className="min-h-0 flex-1 overflow-y-auto px-5 py-4">{bodies[step]()}</div>

          {busy && logTail.length > 0 && (
            <div className="max-h-28 space-y-0.5 overflow-y-auto border-t border-zinc-800 bg-zinc-950 px-5 py-2 font-mono text-[11px] select-text">
              {logTail.map((l, i) => (
                <div key={i} className={/error|failed|exit status/i.test(l.line) ? "text-rose-400" : "text-zinc-400"}>
                  {l.line}
                </div>
              ))}
            </div>
          )}

          <div className="flex items-center justify-between gap-2 border-t border-zinc-800 px-5 py-3">
            <span className="text-[11px] font-mono text-zinc-500">
              {name.trim() ? `kind-${name.trim()} · ${nodeTotal} node${nodeTotal === 1 ? "" : "s"}` : "Name your cluster to continue"}
            </span>
            <div className="flex items-center gap-2">
              <Button variant="ghost" size="sm" className="rounded-none" disabled={busy} onClick={onClose}>
                Cancel
              </Button>
              <Button
                variant="outline"
                size="sm"
                className="rounded-none"
                disabled={busy || step === 0}
                onClick={() => goto(step - 1)}
              >
                Back
              </Button>
              {step < LAST_STEP ? (
                <Button variant="primary" size="sm" className="rounded-none" disabled={busy} onClick={() => goto(step + 1)}>
                  Next
                </Button>
              ) : (
                <Button
                  type="submit"
                  variant="primary"
                  size="sm"
                  className="gap-2 rounded-none"
                  disabled={!canCreate}
                  title={canCreate ? "Create the cluster" : "Fix the errors above first"}
                >
                  {busy ? <Loader2 className="h-4 w-4 animate-spin" /> : <Rocket className="h-4 w-4" />}
                  <span>{busy ? "Creating…" : "Create Cluster"}</span>
                </Button>
              )}
            </div>
          </div>
        </div>
      </form>
    </div>
  );
}
