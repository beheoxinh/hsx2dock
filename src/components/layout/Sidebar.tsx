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

interface NavItem {
  id: NavTab;
  label: string;
  description: string;
  icon: React.ComponentType<{ className?: string }>;
}

interface NavSection {
  title: string;
  items: NavItem[];
}

/** Grouped navigation menu for better visual hierarchy and spacing */
const SECTIONS: NavSection[] = [
  {
    title: "Overview",
    items: [
      {
        id: "dashboard",
        label: "Dashboard",
        description: "Engine metrics & resources",
        icon: LayoutDashboard,
      },
    ],
  },
  {
    title: "Workloads & Storage",
    items: [
      {
        id: "containers",
        label: "Containers",
        description: "Running containers & groups",
        icon: Box,
      },
      {
        id: "images",
        label: "Images",
        description: "Local images & registries",
        icon: Layers,
      },
      {
        id: "volumes",
        label: "Volumes",
        description: "Persistent disk storage",
        icon: HardDrive,
      },
      {
        id: "networks",
        label: "Networks",
        description: "Bridges, overlays & custom",
        icon: Network,
      },
    ],
  },
  {
    title: "Orchestration & Gateway",
    items: [
      {
        id: "compose",
        label: "Compose",
        description: "Multi-service stacks",
        icon: FileCode2,
      },
      {
        id: "kubernetes",
        label: "Kubernetes",
        description: "Kind clusters & node control",
        icon: Hexagon,
      },
      {
        id: "gateway",
        label: "Gateway",
        description: "Caddy HTTPS reverse proxy",
        icon: Globe,
      },
    ],
  },
  {
    title: "System & Tools",
    items: [
      {
        id: "mcp",
        label: "MCP Server",
        description: "AI agent protocol endpoint",
        icon: Bot,
      },
      {
        id: "settings",
        label: "Settings",
        description: "Engine, tools & preferences",
        icon: Settings,
      },
    ],
  },
];

export const Sidebar = React.memo<SidebarProps>(function Sidebar({
  activeTab,
  setActiveTab,
  runningContainersCount = 0,
}) {
  return (
    <aside className="w-80 shrink-0 bg-zinc-950 border-r border-zinc-800/80 flex flex-col h-full select-none">
      {/* Brand / Logo */}
      <div className="h-[4.5rem] flex items-center px-5 gap-3.5 border-b border-zinc-800/70 bg-zinc-900/20">
        <div className="w-10 h-10 rounded-2xl bg-gradient-to-tr from-blue-600 via-indigo-600 to-indigo-500 flex items-center justify-center text-white shadow-lg shadow-blue-500/20 ring-1 ring-white/15">
          <Box className="w-5 h-5 stroke-[2.2]" />
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <h1 className="font-semibold text-[15px] tracking-tight text-white">
              Hsx2Dock
            </h1>
            <span className="text-[10px] font-medium uppercase px-1.5 py-0.5 rounded-none bg-blue-500/10 text-blue-400 border border-blue-500/20">
              Desktop
            </span>
          </div>
          <p className="text-[11.5px] text-zinc-400 font-normal truncate mt-0.5">
            Docker &amp; Container Suite
          </p>
        </div>
      </div>

      {/* Navigation */}
      <nav className="flex-1 py-4 px-3.5 space-y-5 overflow-y-auto custom-scrollbar">
        {SECTIONS.map((section) => (
          <div key={section.title} className="space-y-1.5">
            <div className="px-3 text-[10.5px] font-semibold uppercase tracking-wider text-zinc-400/80">
              {section.title}
            </div>

            <div className="space-y-1">
              {section.items.map((item) => {
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
                      "relative w-full flex items-center justify-between gap-3 rounded-2xl px-3 py-2.5 min-h-[3.75rem] text-left transition-all duration-200 group cursor-pointer focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-blue-500/60 border",
                      isActive
                        ? "text-white border-blue-500/40 bg-blue-600/15 shadow-sm shadow-blue-500/10"
                        : "text-zinc-400 border-transparent hover:border-zinc-800/80 hover:bg-zinc-900/60 hover:text-zinc-100"
                    )}
                  >
                    {isActive && (
                      <motion.div
                        layoutId="active-pill"
                        className="absolute inset-0 rounded-2xl bg-gradient-to-r from-blue-600/20 via-blue-600/10 to-transparent pointer-events-none"
                        transition={{ type: "spring", stiffness: 380, damping: 32 }}
                      />
                    )}

                    <div className="relative z-10 flex min-w-0 items-center gap-3">
                      <span
                        className={cn(
                          "flex h-9 w-9 shrink-0 items-center justify-center rounded-xl transition-all duration-200 shadow-sm",
                          isActive
                            ? "bg-blue-500/25 text-blue-300 ring-1 ring-blue-400/40"
                            : "bg-zinc-900/80 border border-zinc-800/80 text-zinc-400 group-hover:bg-zinc-800 group-hover:text-zinc-100 group-hover:border-zinc-700/80"
                        )}
                      >
                        <Icon className="w-[18px] h-[18px]" />
                      </span>
                      <span className="flex min-w-0 flex-col justify-center">
                        <span
                          className={cn(
                            "truncate text-[13.5px] font-medium tracking-tight leading-snug",
                            isActive ? "text-white" : "text-zinc-200 group-hover:text-white"
                          )}
                        >
                          {item.label}
                        </span>
                        <span
                          className={cn(
                            "truncate text-[11px] leading-relaxed transition-colors",
                            isActive
                              ? "text-blue-200/70"
                              : "text-zinc-400 group-hover:text-zinc-300"
                          )}
                        >
                          {item.description}
                        </span>
                      </span>
                    </div>

                    {badge !== undefined && (
                      <span className="relative z-10 shrink-0 rounded-none border border-emerald-500/40 bg-emerald-500/20 px-2 py-0.5 text-[11px] font-semibold text-emerald-400 shadow-sm shadow-emerald-500/10">
                        {badge}
                      </span>
                    )}
                  </button>
                );
              })}
            </div>
          </div>
        ))}
      </nav>

      {/* Footer status */}
      <div className="p-3.5 border-t border-zinc-800/70 bg-zinc-950/60">
        <div className="flex items-center gap-2.5 px-3 py-2 rounded-xl bg-zinc-900/50 border border-zinc-800/80 text-xs text-zinc-300 shadow-inner">
          <Activity className="w-4 h-4 text-emerald-400 shrink-0" />
          <div className="min-w-0 flex-1 flex flex-col">
            <span className="text-[12px] font-medium text-zinc-200 truncate leading-tight">Rootless Docker</span>
            <span className="text-[10px] text-zinc-400 truncate">Daemon online &amp; healthy</span>
          </div>
        </div>
      </div>
    </aside>
  );
});
