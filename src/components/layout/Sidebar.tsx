import React from "react";
import {
  LayoutDashboard,
  Box,
  Layers,
  HardDrive,
  Network,
  Hexagon,
  FileCode2,
  Settings,
  Activity,
  Bot,
  Globe,
} from "lucide-react";
import { motion } from "framer-motion";
import { cn } from "../../lib/utils";

export type NavTab =
  | "dashboard"
  | "containers"
  | "images"
  | "volumes"
  | "networks"
  | "kubernetes"
  | "compose"
  | "gateway"
  | "mcp"
  | "settings";

interface SidebarProps {
  activeTab: NavTab;
  setActiveTab: (tab: NavTab) => void;
  runningContainersCount?: number;
}

export const Sidebar: React.FC<SidebarProps> = ({
  activeTab,
  setActiveTab,
  runningContainersCount = 0,
}) => {
  const menuItems = [
    { id: "dashboard", label: "Dashboard", icon: LayoutDashboard },
    {
      id: "containers",
      label: "Containers",
      icon: Box,
      badge: runningContainersCount > 0 ? runningContainersCount : undefined,
    },
    { id: "images", label: "Images", icon: Layers },
    { id: "volumes", label: "Volumes", icon: HardDrive },
    { id: "networks", label: "Networks", icon: Network },
    { id: "kubernetes", label: "Kubernetes", icon: Hexagon },
    { id: "compose", label: "Compose", icon: FileCode2 },
    { id: "gateway", label: "Gateway", icon: Globe },
    { id: "mcp", label: "MCP Server", icon: Bot },
    { id: "settings", label: "Settings", icon: Settings },
  ];

  return (
    <aside className="w-64 bg-zinc-950/80 backdrop-blur-2xl border-r border-zinc-800/80 flex flex-col h-full select-none">
      {/* Brand / Logo */}
      <div className="h-16 flex items-center px-6 gap-3 border-b border-zinc-800/60">
        <div className="w-9 h-9 rounded-xl bg-gradient-to-tr from-blue-600 to-indigo-500 flex items-center justify-center text-white shadow-lg shadow-blue-500/25">
          <Box className="w-5 h-5 stroke-[2.2]" />
        </div>
        <div>
          <h1 className="font-semibold text-sm tracking-tight text-white flex items-center gap-1.5">
            Hsx2Dock
          </h1>
          <p className="text-[11px] text-zinc-400 font-medium">Desktop Suite</p>
        </div>
      </div>

      {/* Navigation Links */}
      <nav className="flex-1 py-4 px-3 space-y-1 overflow-y-auto">
        <div className="px-3 pb-2 text-[10px] font-semibold uppercase tracking-wider text-zinc-400">
          Management
        </div>
        {menuItems.map((item) => {
          const Icon = item.icon;
          const isActive = activeTab === item.id;

          return (
            <button
              key={item.id}
              onClick={() => setActiveTab(item.id as NavTab)}
              className={cn(
                "relative w-full flex items-center justify-between px-3.5 py-2.5 rounded-xl text-sm font-medium transition-all group cursor-pointer",
                isActive
                  ? "text-white"
                  : "text-zinc-400 hover:text-zinc-200 hover:bg-zinc-900/50"
              )}
            >
              {isActive && (
                <motion.div
                  layoutId="active-pill"
                  className="absolute inset-0 bg-blue-600/15 border border-blue-500/30 rounded-xl"
                  transition={{ type: "spring", stiffness: 350, damping: 30 }}
                />
              )}

              <div className="flex items-center gap-3 relative z-10">
                <Icon
                  className={cn(
                    "w-4 h-4 transition-colors",
                    isActive
                      ? "text-blue-400"
                      : "text-zinc-400 group-hover:text-zinc-200"
                  )}
                />
                <span>{item.label}</span>
              </div>

              {item.badge !== undefined && (
                <span className="relative z-10 px-2 py-0.5 text-[11px] font-semibold bg-emerald-500/20 text-emerald-400 border border-emerald-500/30 rounded-full">
                  {item.badge}
                </span>
              )}
            </button>
          );
        })}
      </nav>

      {/* Footer Info */}
      <div className="p-4 border-t border-zinc-800/60 bg-zinc-950/40">
        <div className="flex items-center gap-2.5 px-2 py-1.5 rounded-lg bg-zinc-900/40 border border-zinc-800/60 text-xs text-zinc-400">
          <Activity className="w-3.5 h-3.5 text-blue-400 animate-pulse" />
          <span className="truncate">Rootless Engine Active</span>
        </div>
      </div>
    </aside>
  );
};
