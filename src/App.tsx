import React, { useState, useEffect } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { Sidebar, NavTab } from "./components/layout/Sidebar";
import { Header } from "./components/layout/Header";
import { RemoteEndpointBanner } from "./components/layout/RemoteEndpointBanner";
import { ToastProvider, useToast } from "./components/ui/Toast";
import { ModalProvider } from "./components/ui/ModalContext";
import { ContainerDragProvider } from "./components/ContainerDragLayer";
import { DashboardPage } from "./pages/DashboardPage";
import { ContainersPage } from "./pages/ContainersPage";
import { ContainerDetailsDrawer } from "./pages/ContainerDetailsDrawer";
import {
  RunContainerWizard,
  type EditContainerTarget,
} from "./pages/RunContainerWizard";
import { ImagesPage } from "./pages/ImagesPage";
import { VolumesPage } from "./pages/VolumesPage";
import { NetworksPage } from "./pages/NetworksPage";
import { KubernetesPage } from "./pages/KubernetesPage";
import { ComposePage } from "./pages/ComposePage";
import { GatewayPage } from "./pages/GatewayPage";
import { McpPage } from "./pages/McpPage";
import { SettingsPage } from "./pages/SettingsPage";
import { api, events } from "./lib/api";
import { primeContainerStore, startContainerStoreSync } from "./lib/containerStore";
import { primeDashboardStore, resetDashboardEngineData } from "./lib/dashboardStore";
import type { DaemonState } from "./types";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";

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

    // Enforce 90% width and 95% height on application mount
    try {
      const appWin = getCurrentWindow();
      if (window.screen && window.screen.availWidth && window.screen.availHeight) {
        const targetW = Math.round(window.screen.availWidth * 0.90);
        const targetH = Math.round(window.screen.availHeight * 0.95);
        appWin.setSize(new LogicalSize(targetW, targetH)).then(() => {
          appWin.center();
        }).catch((err) => {
          console.warn("Failed to set window size from frontend:", err);
        });
      }
    } catch (err) {
      console.warn("Could not access Tauri window API:", err);
    }

    fetchEngineStatus();

    // Warm the Containers screen before it is ever opened: the list, sizes,
    // groups and gateway routes are fetched once here so the tab paints from
    // memory instead of waiting on Docker on first visit.
    startContainerStoreSync();
    // Warm the Dashboard too: it is the opening screen, so its engine figures,
    // storage donut and tool list should already be in memory when it paints.
    void primeDashboardStore();

    let unlisten: (() => void) | undefined;
    events.onEngineStateChanged((state) => {
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
    }).then((u) => (unlisten = u));

    // A different endpoint is a different daemon: every page must re-fetch,
    // otherwise the UI keeps showing the previous engine's resources.
    let unlistenEndpoint: (() => void) | undefined;
    events
      .onEndpointChanged(() => {
        setRefreshKey((k) => k + 1);
        // A different daemon means a different container list.
        void primeContainerStore(true);
        void primeDashboardStore(true);
      })
      .then((u) => (unlistenEndpoint = u));

    // Tray quick actions. A payload of `kubernetes:<name>` (a cluster row) also
    // asks the Kubernetes page to open that monitor; plain payloads are just
    // navigation. Validate the tab: an old build must never blank the shell.
    let unlistenTray: (() => void) | undefined;
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
    events
      .onTrayNavigate((payload) => {
        const [rawTab, cluster] = payload.split(":", 2);
        if (!NAV_TABS.includes(rawTab as NavTab)) return;
        setTrayCluster(cluster ?? null);
        setActiveTab(rawTab as NavTab);
      })
      .then((u) => (unlistenTray = u));

    return () => {
      if (unlisten) unlisten();
      if (unlistenEndpoint) unlistenEndpoint();
      if (unlistenTray) unlistenTray();
    };
  }, []);

  const handleToggleEngine = async () => {
    try {
      setIsTogglingEngine(true);
      const newState = await api.toggleEngine();
      setDaemonState(newState);
      setRefreshKey((k) => k + 1);
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
            // The refresh button spins and the page data visibly re-renders;
            // a toast here would just be noise.
          }}
        />

        <RemoteEndpointBanner />

        {/* Dynamic Page Views with Smooth Framer Motion Spring Transitions */}
        <main className="flex-1 overflow-hidden relative">
          <AnimatePresence mode="wait">
            <motion.div
              key={activeTab + refreshKey}
              initial={{ opacity: 0, y: 10 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: -10 }}
              transition={{ duration: 0.18, ease: "easeInOut" }}
              className="h-full w-full"
            >
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
            </motion.div>
          </AnimatePresence>
        </main>
      </div>

      {/* Slide-over Container Details Drawer */}
      {selectedContainerId && (
        <ContainerDetailsDrawer
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
      )}

      {/* Run / Edit Container Wizard Modal */}
      <RunContainerWizard
        isOpen={isWizardOpen}
        editTarget={wizardEditTarget}
        onClose={() => {
          setIsWizardOpen(false);
          setWizardEditTarget(null);
        }}
        onSuccess={(newId) => {
          setRefreshKey((k) => k + 1);
          if (wizardEditTarget && selectedContainerId === wizardEditTarget.id) {
            if (newId) {
              setSelectedContainerId(newId);
            } else {
              setSelectedContainerId(null);
            }
          }
        }}
      />
    </div>
  );
};

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
