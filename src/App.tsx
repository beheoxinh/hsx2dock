import React, { useState, useEffect } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { Sidebar, NavTab } from "./components/layout/Sidebar";
import { Header } from "./components/layout/Header";
import { RemoteEndpointBanner } from "./components/layout/RemoteEndpointBanner";
import { ToastProvider, useToast } from "./components/ui/Toast";
import { ModalProvider } from "./components/ui/ModalContext";
import { DashboardPage } from "./pages/DashboardPage";
import { ContainersPage } from "./pages/ContainersPage";
import { ContainerDetailsDrawer } from "./pages/ContainerDetailsDrawer";
import { RunContainerWizard } from "./pages/RunContainerWizard";
import { ImagesPage } from "./pages/ImagesPage";
import { VolumesPage } from "./pages/VolumesPage";
import { NetworksPage } from "./pages/NetworksPage";
import { KubernetesPage } from "./pages/KubernetesPage";
import { ComposePage } from "./pages/ComposePage";
import { SettingsPage } from "./pages/SettingsPage";
import { api, events } from "./lib/api";
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
  const [refreshKey, setRefreshKey] = useState(0);
  const toast = useToast();

  const fetchEngineStatus = async () => {
    try {
      const st = await api.getEngineStatus();
      setDaemonState(st);
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

    let unlisten: (() => void) | undefined;
    events.onEngineStateChanged((state) => {
      setDaemonState(state);
      // Every engine-backed page re-fetches: data appears/disappears with the
      // daemon instead of lingering stale or erroring behind the scenes.
      setRefreshKey((k) => k + 1);
      if (state === "Running") {
        toast.success("Docker Daemon Ready", "Rootless engine daemon is running.");
      } else if (state === "Stopped") {
        toast.info("Docker Daemon Stopped", "Rootless engine has shut down.");
      }
    }).then((u) => (unlisten = u));

    // A different endpoint is a different daemon: every page must re-fetch,
    // otherwise the UI keeps showing the previous engine's resources.
    let unlistenEndpoint: (() => void) | undefined;
    events
      .onEndpointChanged(() => {
        setRefreshKey((k) => k + 1);
      })
      .then((u) => (unlistenEndpoint = u));

    // Tray quick actions ("Containers", "Images", "Manage Kubernetes").
    // Validate the payload: the backend and the shell must stay in sync even
    // if an old build sends a tab this build no longer has.
    let unlistenTray: (() => void) | undefined;
    const NAV_TABS: NavTab[] = [
      "dashboard",
      "containers",
      "images",
      "volumes",
      "networks",
      "kubernetes",
      "compose",
      "settings",
    ];
    events
      .onTrayNavigate((tab) => {
        if (NAV_TABS.includes(tab as NavTab)) setActiveTab(tab as NavTab);
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

  return (
    <div className="flex h-screen w-screen overflow-hidden bg-zinc-950 font-sans">
      {/* Sidebar Navigation */}
      <Sidebar activeTab={activeTab} setActiveTab={setActiveTab} />

      {/* Main Content Area */}
      <div className="flex-1 flex flex-col min-w-0 h-full overflow-hidden">
        {/* Top Header */}
        <Header
          daemonState={daemonState}
          onToggleEngine={handleToggleEngine}
          isToggling={isTogglingEngine}
          onRefresh={() => {
            setRefreshKey((k) => k + 1);
            toast.info("Data refreshed", "Updated dashboard state.");
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
                  onOpenWizard={() => setIsWizardOpen(true)}
                />
              )}
              {activeTab === "images" && <ImagesPage refreshKey={refreshKey} daemonState={daemonState} />}
              {activeTab === "volumes" && <VolumesPage refreshKey={refreshKey} daemonState={daemonState} />}
              {activeTab === "networks" && <NetworksPage refreshKey={refreshKey} daemonState={daemonState} />}
              {activeTab === "kubernetes" && <KubernetesPage refreshKey={refreshKey} daemonState={daemonState} />}
              {activeTab === "compose" && <ComposePage daemonState={daemonState} />}
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
          onClose={() => {
            setSelectedContainerId(null);
            setDrawerDebug(false);
            setDrawerEdit(false);
          }}
        />
      )}

      {/* Run Container Wizard Modal */}
      <RunContainerWizard
        isOpen={isWizardOpen}
        onClose={() => setIsWizardOpen(false)}
        onSuccess={() => {
          setRefreshKey((k) => k + 1);
          toast.success("Container Launched", "New container instance created.");
        }}
      />
    </div>
  );
};

export const App: React.FC = () => {
  return (
    <ToastProvider>
      <ModalProvider>
        <MainLayout />
      </ModalProvider>
    </ToastProvider>
  );
};
