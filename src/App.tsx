import React, { useState, useEffect, Suspense } from "react";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { motion, AnimatePresence } from "framer-motion";
import { Sidebar, NavTab } from "./components/layout/Sidebar";
import { Header } from "./components/layout/Header";
import { RemoteEndpointBanner } from "./components/layout/RemoteEndpointBanner";
import { ToastProvider, useToast } from "./components/ui/Toast";
import { ModalProvider } from "./components/ui/ModalContext";
import { ContainerDragProvider } from "./components/ContainerDragLayer";
import { DashboardPage } from "./pages/DashboardPage";
import { ContainersPage } from "./pages/ContainersPage";
import type { EditContainerTarget } from "./pages/RunContainerWizard";

const ContainerDetailsDrawer = React.lazy(() =>
  import("./pages/ContainerDetailsDrawer").then((m) => ({ default: m.ContainerDetailsDrawer }))
);
const RunContainerWizard = React.lazy(() =>
  import("./pages/RunContainerWizard").then((m) => ({ default: m.RunContainerWizard }))
);
const ImagesPage = React.lazy(() =>
  import("./pages/ImagesPage").then((m) => ({ default: m.ImagesPage }))
);
const VolumesPage = React.lazy(() =>
  import("./pages/VolumesPage").then((m) => ({ default: m.VolumesPage }))
);
const NetworksPage = React.lazy(() =>
  import("./pages/NetworksPage").then((m) => ({ default: m.NetworksPage }))
);
const KubernetesPage = React.lazy(() =>
  import("./pages/KubernetesPage").then((m) => ({ default: m.KubernetesPage }))
);
const ComposePage = React.lazy(() =>
  import("./pages/ComposePage").then((m) => ({ default: m.ComposePage }))
);
const GatewayPage = React.lazy(() =>
  import("./pages/GatewayPage").then((m) => ({ default: m.GatewayPage }))
);
const McpPage = React.lazy(() =>
  import("./pages/McpPage").then((m) => ({ default: m.McpPage }))
);
const SettingsPage = React.lazy(() =>
  import("./pages/SettingsPage").then((m) => ({ default: m.SettingsPage }))
);
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api, disposeListener, events } from "./lib/api";
import { primeContainerStore, startContainerStoreSync } from "./lib/containerStore";
import { startContainerOpsSync, subscribeOpNotices } from "./lib/containerOps";
import { primeDashboardStore, resetDashboardEngineData } from "./lib/dashboardStore";
import { alertKindLabel } from "./lib/clusterAlerts";
import type { ClusterAlertsEvent, DaemonState } from "./types";

const MainLayout: React.FC = () => {
  const [activeTab, setActiveTab] = useState<NavTab>("dashboard");
  const [daemonState, setDaemonState] = useState<DaemonState>("Stopped");
  const [isTogglingEngine, setIsTogglingEngine] = useState(false);
  const [selectedContainerId, setSelectedContainerId] = useState<string | null>(null);
  const [drawerTab, setDrawerTab] = useState<"logs" | "inspect" | "stats" | "terminal">("logs");
  const [drawerDebug, setDrawerDebug] = useState(false);
  const [drawerEdit, setDrawerEdit] = useState(false);
  const [isWizardOpen, setIsWizardOpen] = useState(false);
  const [wizardEditTarget, setWizardEditTarget] = useState<EditContainerTarget | null>(null);
  const [refreshKey, setRefreshKey] = useState(0);
  // Cluster requested by a tray row click; consumed by KubernetesPage.
  const [trayCluster, setTrayCluster] = useState<string | null>(null);
  const toast = useToast();

  const fetchEngineStatus = async () => {
    try {
      const st = await api.getEngineStatus();
      setDaemonState(st);
      // A stopped daemon invalidates every cached engine figure — drop the
      // hydrated dashboard cache instead of showing yesterday's numbers.
      if (st !== "Running") resetDashboardEngineData();
    } catch (e) {
      console.error("Failed to fetch engine status:", e);
    }
  };

  useEffect(() => {
    // Initialize Theme
    const savedTheme = localStorage.getItem("theme");
    if (savedTheme === "dark" || (!savedTheme && window.matchMedia("(prefers-color-scheme: dark)").matches)) {
      document.documentElement.classList.add("dark");
      document.documentElement.classList.remove("light");
    } else if (savedTheme === "light") {
      document.documentElement.classList.remove("dark");
      document.documentElement.classList.add("light");
    }

    // The startup window size is set once by the backend (lib.rs). Doing it
    // here as well made the window jump and undid the user's own resize on
    // every remount.

    fetchEngineStatus();

    // A webview reload drops every terminal without telling the backend, so
    // their exec sessions (and the shells inside containers) lived on. This
    // is the only window, so anything still open now belongs to a dead page.
    api.closeAllExecSessions().catch(() => {});

    // Warm the Containers screen before it is ever opened: the list, sizes,
    // groups and gateway routes are fetched once here so the tab paints from
    // memory instead of waiting on Docker on first visit.
    startContainerStoreSync();
    // Container/group operations in flight (group starts, auto-start on
    // engine boot, tray actions) drive the per-card spinners; failures that
    // happen in the background are surfaced here.
    startContainerOpsSync();
    const stopNotices = subscribeOpNotices((n) => toast.error(n.title, n.detail));
    // Warm the Dashboard too: it is the opening screen, so its engine figures,
    // storage donut and tool list should already be in memory when it paints.
    void primeDashboardStore();

    const unlisten = disposeListener(events.onEngineStateChanged((state) => {
      setDaemonState(state);
      // Every engine-backed page re-fetches: data appears/disappears with the
      // daemon instead of lingering stale or erroring behind the scenes.
      setRefreshKey((k) => k + 1);
      if (state === "Running") {
        // The container cache belongs to the daemon that produced it.
        void primeContainerStore(true);
        void primeDashboardStore(true);
      } else {
        // Stopped: engine-backed dashboard figures no longer describe anything
        // that is running, so drop them instead of showing stale numbers.
        resetDashboardEngineData();
      }
      // Engine start/stop is visible in the header and every page gate, so it
      // is not announced with a toast; only failures are.
    }));

    // A different endpoint is a different daemon: every page must re-fetch,
    // otherwise the UI keeps showing the previous engine's resources.
    const unlistenEndpoint = disposeListener(
      events.onEndpointChanged(() => {
        setRefreshKey((k) => k + 1);
        // A different daemon means a different container list.
        void primeContainerStore(true);
        void primeDashboardStore(true);
      })
    );

    // Tray quick actions. A payload of `kubernetes:<name>` (a cluster row) also
    // asks the Kubernetes page to open that monitor; plain payloads are just
    // navigation. Validate the tab: an old build must never blank the shell.
    const NAV_TABS: NavTab[] = [
      "dashboard",
      "containers",
      "images",
      "volumes",
      "networks",
      "kubernetes",
      "compose",
      "gateway",
      "mcp",
      "settings",
    ];
    const unlistenTray = disposeListener(
      events.onTrayNavigate((payload) => {
        const [rawTab, cluster] = payload.split(":", 2);
        if (!NAV_TABS.includes(rawTab as NavTab)) return;
        setTrayCluster(cluster ?? null);
        setActiveTab(rawTab as NavTab);
      })
    );

    // Cluster incident alerts: one plain toast per newly raised alert.
    const unlistenAlerts = disposeListener(
      listen<ClusterAlertsEvent>("cluster-alerts", (e) => {
        for (const a of e.payload.fresh) {
          const title = `${alertKindLabel(a.kind)}: ${a.object}`;
          const description = `${a.cluster}${a.namespace ? "/" + a.namespace : ""} — ${a.message}`;
          if (a.severity === "error") toast.error(title, description);
          else toast.info(title, description);
        }
      })
    );

    // Focusing the window after an alert click-through opens the cluster it names.
    const unlistenFocus = disposeListener(
      getCurrentWindow().onFocusChanged(async (e) => {
        if (!e.payload) return;
        const name = await api.takeAlertFocus().catch(() => null);
        if (!name) return;
        setTrayCluster(name);
        setActiveTab("kubernetes");
        window.dispatchEvent(new CustomEvent("hsx:open-cluster", { detail: name }));
      })
    );

    return () => {
      unlisten();
      unlistenEndpoint();
      unlistenTray();
      stopNotices();
      unlistenAlerts();
      unlistenFocus();
    };
  }, []);

  const handleToggleEngine = async () => {
    try {
      setIsTogglingEngine(true);
      const newState = await api.toggleEngine();
      setDaemonState(newState);
      // Re-verify against the backend: `toggle_engine` may return Running from
      // a process that has not opened its socket yet, and a failed start must
      // not leave a stale "Running" label on the button.
      const verified = await api.getEngineStatus();
      setDaemonState(verified);
      if (verified === "Running") {
        void primeContainerStore(true);
        void primeDashboardStore(true);
      }
    } catch (e) {
      toast.error("Failed to toggle engine", String(e));
      // The daemon may have died mid-startup; never leave a stale "Running"
      // button behind — re-read the real state from the backend.
      try {
        setDaemonState(await api.getEngineStatus());
      } catch {
        setDaemonState("Stopped");
      }
    } finally {
      setIsTogglingEngine(false);
    }
  };

  const handleOpenDetails = (
    id: string,
    tab: "logs" | "inspect" | "stats" | "terminal" = "logs",
    debug = false,
    edit = false
  ) => {
    setSelectedContainerId(id);
    setDrawerTab(tab);
    setDrawerDebug(debug);
    setDrawerEdit(edit);
  };

  const handleOpenEditContainer = async (containerId: string) => {
    try {
      const details = await api.inspectContainer(containerId);
      setWizardEditTarget({ id: containerId, details });
      setIsWizardOpen(true);
    } catch (err) {
      toast.error("Cannot inspect container", String(err));
    }
  };

  return (
    <div className="flex h-screen w-screen overflow-hidden bg-zinc-950 font-sans">
      {/* Sidebar Navigation */}
      <Sidebar
        activeTab={activeTab}
        setActiveTab={(tab) => {
          // Manual navigation always lands on the page's normal entry point,
          // not the cluster a previous tray click opened.
          setTrayCluster(null);
          setActiveTab(tab);
        }}
      />

      {/* Main Content Area */}
      <div className="flex-1 flex flex-col min-w-0 h-full overflow-hidden">
        {/* Top Header */}
        <Header
          daemonState={daemonState}
          onToggleEngine={handleToggleEngine}
          isToggling={isTogglingEngine}
          onOpenClusters={() => {
            setTrayCluster(null);
            setActiveTab("kubernetes");
          }}
          onRefresh={() => {
            setRefreshKey((k) => k + 1);
            void primeContainerStore(true);
            void primeDashboardStore(true);
            // The refresh button spins and the page data visibly re-renders;
            // a toast here would just be noise.
          }}
        />

        <RemoteEndpointBanner />

        {/* Dynamic Page Views with Smooth Framer Motion Spring Transitions */}
        <main className="flex-1 overflow-hidden relative">
          <AnimatePresence mode="wait">
            <motion.div
              // Pages that refetch on `refreshKey` (or live in a store) keep
              // their state across engine changes and manual refreshes; a
              // remount threw away open modals, filters and scroll.
              key={REFRESH_IN_PLACE.has(activeTab) ? activeTab : `${activeTab}:${refreshKey}`}
              initial={{ opacity: 0, y: 10 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: -10 }}
              transition={{ duration: 0.18, ease: "easeInOut" }}
              className="h-full w-full"
            >
              <ErrorBoundary key={activeTab} label="This page">
              <Suspense fallback={<div className="h-full w-full flex items-center justify-center text-xs text-zinc-500">Loading…</div>}>
                {activeTab === "dashboard" && (
                  <DashboardPage
                    daemonState={daemonState}
                    onNavigate={(tab) => setActiveTab(tab)}
                  />
                )}
                {activeTab === "containers" && (
                  <ContainersPage
                    daemonState={daemonState}
                    onOpenDetails={handleOpenDetails}
                    onOpenWizard={() => {
                      setWizardEditTarget(null);
                      setIsWizardOpen(true);
                    }}
                    onEditContainer={handleOpenEditContainer}
                  />
                )}
                {activeTab === "images" && <ImagesPage refreshKey={refreshKey} daemonState={daemonState} />}
                {activeTab === "volumes" && <VolumesPage refreshKey={refreshKey} daemonState={daemonState} />}
                {activeTab === "networks" && <NetworksPage refreshKey={refreshKey} daemonState={daemonState} />}
                {activeTab === "kubernetes" && (
                  <KubernetesPage
                    refreshKey={refreshKey}
                    daemonState={daemonState}
                    openCluster={trayCluster}
                  />
                )}
                {activeTab === "compose" && <ComposePage daemonState={daemonState} />}
                {activeTab === "gateway" && <GatewayPage daemonState={daemonState} />}
                {activeTab === "mcp" && <McpPage />}
                {activeTab === "settings" && <SettingsPage />}
              </Suspense>
              </ErrorBoundary>
            </motion.div>
          </AnimatePresence>
        </main>
      </div>

      {/* Slide-over Container Details Drawer */}
      {selectedContainerId && (
        <Suspense fallback={null}>
          <ContainerDetailsDrawer
            key={selectedContainerId}
            containerId={selectedContainerId}
            initialTab={drawerTab}
            debug={drawerDebug}
            editLimits={drawerEdit}
            onOpenEditConfig={(details) => {
              setWizardEditTarget({ id: details.id, details });
              setIsWizardOpen(true);
            }}
            onClose={() => {
              setSelectedContainerId(null);
              setDrawerDebug(false);
              setDrawerEdit(false);
            }}
          />
        </Suspense>
      )}

      {/* Run / Edit Container Wizard Modal */}
      {isWizardOpen && (
        <Suspense fallback={null}>
          <RunContainerWizard
            isOpen={isWizardOpen}
            editTarget={wizardEditTarget}
            onClose={() => {
              setIsWizardOpen(false);
              setWizardEditTarget(null);
            }}
            onSuccess={(newId) => {
              setRefreshKey((k) => k + 1);
              // Recreate swaps the container id inside its group on the
              // backend; reload list and groups so the next group save does
              // not write the stale id back.
              void primeContainerStore(true);
              if (wizardEditTarget && selectedContainerId === wizardEditTarget.id) {
                if (newId) {
                  setSelectedContainerId(newId);
                } else {
                  setSelectedContainerId(null);
                }
              }
            }}
          />
        </Suspense>
      )}
    </div>
  );
};

/** Tabs whose data follows `refreshKey` or a store instead of a remount. */
const REFRESH_IN_PLACE = new Set([
  "dashboard",
  "containers",
  "images",
  "volumes",
  "networks",
  "kubernetes",
]);

export const App: React.FC = () => {
  return (
    <ToastProvider>
      <ModalProvider>
        <ContainerDragProvider>
          <MainLayout />
        </ContainerDragProvider>
      </ModalProvider>
    </ToastProvider>
  );
};
