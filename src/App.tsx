import React, { useState, useEffect } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { Sidebar, NavTab } from "./components/layout/Sidebar";
import { Header } from "./components/layout/Header";
import { ToastProvider, useToast } from "./components/ui/Toast";
import { DashboardPage } from "./pages/DashboardPage";
import { ContainersPage } from "./pages/ContainersPage";
import { ContainerDetailsDrawer } from "./pages/ContainerDetailsDrawer";
import { RunContainerWizard } from "./pages/RunContainerWizard";
import { ImagesPage } from "./pages/ImagesPage";
import { VolumesPage } from "./pages/VolumesPage";
import { NetworksPage } from "./pages/NetworksPage";
import { ComposePage } from "./pages/ComposePage";
import { SettingsPage } from "./pages/SettingsPage";
import { api, events } from "./lib/api";
import type { DaemonState } from "./types";

const MainLayout: React.FC = () => {
  const [activeTab, setActiveTab] = useState<NavTab>("dashboard");
  const [daemonState, setDaemonState] = useState<DaemonState>("Stopped");
  const [isTogglingEngine, setIsTogglingEngine] = useState(false);
  const [selectedContainerId, setSelectedContainerId] = useState<string | null>(null);
  const [drawerTab, setDrawerTab] = useState<"logs" | "inspect" | "stats">("logs");
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
    fetchEngineStatus();

    let unlisten: (() => void) | undefined;
    events.onEngineStateChanged((state) => {
      setDaemonState(state);
      if (state === "Running") {
        toast.success("Docker Daemon Ready", "Rootless engine daemon is running.");
      } else if (state === "Stopped") {
        toast.info("Docker Daemon Stopped", "Rootless engine has shut down.");
      }
    }).then((u) => (unlisten = u));

    return () => {
      if (unlisten) unlisten();
    };
  }, []);

  const handleToggleEngine = async () => {
    try {
      setIsTogglingEngine(true);
      const newState = await api.toggleEngine();
      setDaemonState(newState);
      setRefreshKey((k) => k + 1);
    } catch (e) {
      toast.error("Failed to toggle engine", String(e));
    } finally {
      setIsTogglingEngine(false);
    }
  };

  const handleOpenDetails = (id: string, tab: "logs" | "inspect" | "stats" = "logs") => {
    setSelectedContainerId(id);
    setDrawerTab(tab);
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
                  onOpenDetails={handleOpenDetails}
                  onOpenWizard={() => setIsWizardOpen(true)}
                />
              )}
              {activeTab === "images" && <ImagesPage refreshKey={refreshKey} />}
              {activeTab === "volumes" && <VolumesPage refreshKey={refreshKey} />}
              {activeTab === "networks" && <NetworksPage refreshKey={refreshKey} />}
              {activeTab === "compose" && <ComposePage />}
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
          onClose={() => setSelectedContainerId(null)}
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
      <MainLayout />
    </ToastProvider>
  );
};
