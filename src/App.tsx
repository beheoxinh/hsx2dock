import React, { useState, useEffect } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { Sidebar, NavTab } from "./components/layout/Sidebar";
import { Header } from "./components/layout/Header";
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

export const App: React.FC = () => {
  const [activeTab, setActiveTab] = useState<NavTab>("dashboard");
  const [daemonState, setDaemonState] = useState<DaemonState>("Stopped");
  const [isTogglingEngine, setIsTogglingEngine] = useState(false);
  const [selectedContainerId, setSelectedContainerId] = useState<string | null>(null);
  const [drawerTab, setDrawerTab] = useState<"logs" | "inspect" | "stats">("logs");
  const [isWizardOpen, setIsWizardOpen] = useState(false);
  const [refreshKey, setRefreshKey] = useState(0);

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

    // Listen to real-time engine state changes from backend
    let unlisten: (() => void) | undefined;
    events.onEngineStateChanged((state) => {
      setDaemonState(state);
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
      alert(`Toggle Engine Error: ${e}`);
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
      {/* Sidebar */}
      <Sidebar activeTab={activeTab} setActiveTab={setActiveTab} />

      {/* Main Content Area */}
      <div className="flex-1 flex flex-col min-w-0 h-full overflow-hidden">
        {/* Top Header */}
        <Header
          daemonState={daemonState}
          onToggleEngine={handleToggleEngine}
          isToggling={isTogglingEngine}
          onRefresh={() => setRefreshKey((k) => k + 1)}
        />

        {/* Dynamic Page View with Spring Physics Animations */}
        <main className="flex-1 overflow-hidden relative">
          <AnimatePresence mode="wait">
            <motion.div
              key={activeTab + refreshKey}
              initial={{ opacity: 0, y: 8 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: -8 }}
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
              {activeTab === "images" && <ImagesPage />}
              {activeTab === "volumes" && <VolumesPage />}
              {activeTab === "networks" && <NetworksPage />}
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
        onSuccess={() => setRefreshKey((k) => k + 1)}
      />
    </div>
  );
};
