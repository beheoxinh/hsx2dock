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

/** One row per destination: label plus the one-line "what is this?" hint. */
const MENU: Array<{
  id: NavTab;
  label: string;
  description: string;
  icon: React.ComponentType<{ className?: string }>;
}> = [
  {
    id: "dashboard",
    label: "Dashboard",
    description: "Engine health & usage",
    icon: LayoutDashboard,
  },
  {
    id: "containers",
    label: "Containers",
    description: "Running containers & groups",
    icon: Box,
  },
  {
    id: "images",
    label: "Images",
    description: "Local images, builds & pulls",
    icon: Layers,
  },
  {
    id: "volumes",
    label: "Volumes",
    description: "Persistent storage",
    icon: HardDrive,
  },
  {
    id: "networks",
    label: "Networks",
    description: "Bridges & custom networks",
    icon: Network,
  },
  {
    id: "kubernetes",
    label: "Kubernetes",
    description: "Local kind clusters",
    icon: Hexagon,
  },
  {
    id: "compose",
    label: "Compose",
    description: "Multi-service projects",
    icon: FileCode2,
  },
  {
    id: "gateway",
    label: "Gateway",
    description: "HTTPS virtual hostnames",
    icon: Globe,
  },
  {
    id: "mcp",
    label: "MCP Server",
    description: "Automation endpoint",
    icon: Bot,
  },
  {
    id: "settings",
    label: "Settings",
    description: "Engine & app preferences",
    icon: Settings,
  },
];

export const Sidebar: React.FC<SidebarProps> = ({
  activeTab,
  setActiveTab,
  runningContainersCount = 0,
}) => {
  return (
    <aside className="w-[17.5rem] shrink-0 bg-zinc-950/80 backdrop-blur-2xl border-r border-zinc-800/80 flex flex-col h-full select-none">
      {/* Brand / Logo */}
      <div className="h-16 flex items-center px-5 gap-3 border-b border-zinc-800/60">
        <div className="w-9 h-9 rounded-xl bg-gradient-to-tr from-blue-600 to-indigo-500 flex items-center justify-center text-white shadow-lg shadow-blue-500/25">
          <Box className="w-5 h-5 stroke-[2.2]" />
        </div>
        <div>
          <h1 className="font-semibold text-sm tracking-tight text-white flex items-center gap-1.5">
            Hsx2Dock
          </h1>
          <p className="text-[11px] text-zinc-500 font-medium">Desktop Suite</p>
        </div>
      </div>

      {/* Navigation */}
      <nav className="flex-1 py-4 px-3 space-y-1 overflow-y-auto">
        <div className="px-3 pb-2 text-[10px] font-semibold uppercase tracking-wider text-zinc-500">
          Management
        </div>
        {MENU.map((item) => {
          const Icon = item.icon;
          const isActive = activeTab === item.id;
          const badge =
            item.id === "containers" && runningContainersCount > 0
              ? runningContainersCount
              : undefined;

          return (
            <button
              key={item.id}
              type="button"
              onClick={() => setActiveTab(item.id)}
              aria-current={isActive ? "page" : undefined}
              className={cn(
                "relative w-full flex items-center justify-between gap-2 rounded-xl px-2.5 py-2 text-left transition-colors group cursor-pointer focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-blue-500/60",
                isActive ? "text-white" : "text-zinc-400 hover:text-zinc-100"
              )}
            >
              {isActive && (
                <motion.div
                  layoutId="active-pill"
                  className="absolute inset-0 rounded-xl bg-blue-600/15 border border-blue-500/30"
                  transition={{ type: "spring", stiffness: 380, damping: 32 }}
                />
              )}

              <div className="relative z-10 flex min-w-0 items-center gap-2.5">
                <span
                  className={cn(
                    "flex h-8 w-8 shrink-0 items-center justify-center rounded-lg transition-colors",
                    isActive
                      ? "bg-blue-500/20 text-blue-300"
                      : "bg-zinc-900/60 text-zinc-400 group-hover:bg-zinc-800/70 group-hover:text-zinc-200"
                  )}
                >
                  <Icon className="w-[17px] h-[17px]" />
                </span>
                <span className="flex min-w-0 flex-col leading-tight">
                  <span
                    className={cn(
                      "truncate text-[13.5px] font-medium tracking-tight",
                      isActive ? "text-white" : "text-zinc-300 group-hover:text-white"
                    )}
                  >
                    {item.label}
                  </span>
                  <span className="truncate text-[10.5px] font-normal text-zinc-500 group-hover:text-zinc-400">
                    {item.description}
                  </span>
                </span>
              </div>

              {badge !== undefined && (
                <span className="relative z-10 shrink-0 rounded-full border border-emerald-500/30 bg-emerald-500/20 px-1.5 py-0.5 text-[10.5px] font-semibold text-emerald-400">
                  {badge}
                </span>
              )}
            </button>
          );
        })}
      </nav>

      {/* Footer status */}
      <div className="p-3 border-t border-zinc-800/60 bg-zinc-950/40">
        <div className="flex items-center gap-2.5 px-2 py-1.5 rounded-lg bg-zinc-900/40 border border-zinc-800/60 text-xs text-zinc-400">
          <Activity className="w-3.5 h-3.5 text-blue-400 animate-pulse" />
          <span className="truncate">Rootless Engine Active</span>
        </div>
      </div>
    </aside>
  );
};
