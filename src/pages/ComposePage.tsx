import { useToast } from "../components/ui/Toast";
import React, { useEffect, useState } from "react";
import { FileCode2, Play, Square, Terminal, RefreshCw } from "lucide-react";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { StatusBadge } from "../components/ui/StatusBadge";
import { api } from "../lib/api";
import type { ComposeProjectInfo } from "../types";

export const ComposePage: React.FC = () => {
  const toast = useToast();
  const [projects, setProjects] = useState<ComposeProjectInfo[]>([]);
  const [, setLoading] = useState(true);
  const [selectedProject, setSelectedProject] = useState<ComposeProjectInfo | null>(null);
  const [logs, setLogs] = useState("");
  const [logsLoading, setLogsLoading] = useState(false);
  const [actionLoading, setActionLoading] = useState(false);

  const loadProjects = async () => {
    try {
      setLoading(true);
      const list = await api.listComposeProjects();
      setProjects(list);
      if (list.length > 0 && !selectedProject) {
        setSelectedProject(list[0]);
      }
    } catch (e) {
      console.error(e);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    loadProjects();
  }, []);

  const loadLogs = async (path: string) => {
    try {
      setLogsLoading(true);
      const out = await api.composeLogs(path, "150");
      setLogs(out);
    } catch (e) {
      setLogs(`Error reading compose logs: ${e}`);
    } finally {
      setLogsLoading(false);
    }
  };

  useEffect(() => {
    if (selectedProject) {
      loadLogs(selectedProject.path);
    }
  }, [selectedProject]);

  const handleUp = async (path: string) => {
    try {
      setActionLoading(true);
      await api.composeUp(path);
      await loadProjects();
      await loadLogs(path);
    } catch (e) {
      toast.error("Compose Up Error", String(e));
    } finally {
      setActionLoading(false);
    }
  };

  const handleDown = async (path: string) => {
    try {
      setActionLoading(true);
      await api.composeDown(path);
      await loadProjects();
      await loadLogs(path);
    } catch (e) {
      toast.error("Compose Down Error", String(e));
    } finally {
      setActionLoading(false);
    }
  };

  return (
    <div className="p-8 space-y-6 max-w-7xl mx-auto h-full flex flex-col">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h2 className="text-2xl font-bold tracking-tight text-white">Docker Compose</h2>
          <p className="text-sm text-zinc-400 mt-1">
            Orchestrate multi-container applications and examine combined logs
          </p>
        </div>
        <Button variant="ghost" size="icon" onClick={loadProjects} title="Reload">
          <RefreshCw className="w-4 h-4" />
        </Button>
      </div>

      {projects.length > 0 ? (
        <div className="grid grid-cols-1 lg:grid-cols-3 gap-6 flex-1 min-h-0">
          {/* Projects and Services List */}
          <div className="space-y-4 overflow-y-auto">
            {projects.map((proj) => (
              <Card
                key={proj.path}
                className={`cursor-pointer transition-all ${
                  selectedProject?.path === proj.path
                    ? "border-blue-500/60 bg-blue-500/5"
                    : "hover:border-zinc-700/60"
                }`}
                onClick={() => setSelectedProject(proj)}
              >
                <div className="flex flex-wrap items-center justify-between gap-3">
                  <div className="flex items-center gap-3">
                    <div className="w-9 h-9 rounded-xl bg-blue-500/10 border border-blue-500/20 flex items-center justify-center text-blue-400">
                      <FileCode2 className="w-4 h-4" />
                    </div>
                    <div>
                      <h4 className="font-semibold text-white">{proj.name}</h4>
                      <p className="text-xs text-zinc-400 truncate max-w-[200px]" title={proj.path}>
                        {proj.path}
                      </p>
                    </div>
                  </div>

                  <div className="flex items-center gap-1.5">
                    <Button
                      variant="ghost"
                      size="icon"
                      onClick={(e) => {
                        e.stopPropagation();
                        handleUp(proj.path);
                      }}
                      disabled={actionLoading}
                      title="Compose Up"
                      className="text-emerald-400 hover:text-emerald-300"
                    >
                      <Play className="w-4 h-4 fill-current" />
                    </Button>
                    <Button
                      variant="ghost"
                      size="icon"
                      onClick={(e) => {
                        e.stopPropagation();
                        handleDown(proj.path);
                      }}
                      disabled={actionLoading}
                      title="Compose Down"
                      className="text-rose-400 hover:text-rose-300"
                    >
                      <Square className="w-4 h-4 fill-current" />
                    </Button>
                  </div>
                </div>

                {/* Services summary */}
                <div className="mt-4 pt-3 border-t border-zinc-800/60 space-y-2">
                  <span className="text-[10px] uppercase font-semibold text-zinc-400 tracking-wider">
                    Services ({proj.services.length})
                  </span>
                  <div className="space-y-1.5">
                    {proj.services.map((svc) => (
                      <div
                        key={svc.id || svc.name}
                        className="flex items-center justify-between text-xs py-1 px-2 rounded-lg bg-zinc-900/60 border border-zinc-800/40"
                      >
                        <span className="font-medium text-zinc-200">{svc.service}</span>
                        <StatusBadge status={svc.state} />
                      </div>
                    ))}
                  </div>
                </div>
              </Card>
            ))}
          </div>

          {/* Project Logs Viewer */}
          <Card className="lg:col-span-2 flex flex-col min-h-0 overflow-hidden">
            <div className="flex items-center justify-between pb-3 border-b border-zinc-800">
              <div className="flex items-center gap-2">
                <Terminal className="w-4 h-4 text-blue-400" />
                <span className="font-semibold text-white text-sm">
                  Logs: {selectedProject?.name}
                </span>
              </div>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => selectedProject && loadLogs(selectedProject.path)}
                isLoading={logsLoading}
                className="text-xs gap-1.5"
              >
                <RefreshCw className="w-3.5 h-3.5" />
                <span>Refresh</span>
              </Button>
            </div>

            <div className="flex-1 bg-zinc-950 rounded-xl border border-zinc-800/80 p-4 mt-3 overflow-hidden flex flex-col">
              <pre className="flex-1 font-mono text-xs text-zinc-300 overflow-y-auto whitespace-pre-wrap select-text leading-relaxed">
                {logs || "No logs output yet. Click 'Play' to spin up the Compose stack."}
              </pre>
            </div>
          </Card>
        </div>
      ) : (
        <div className="py-24 text-center text-zinc-400">
          <FileCode2 className="w-12 h-12 mx-auto text-zinc-400 mb-3" />
          <h3 className="font-semibold text-lg text-white">No Compose Project Configured</h3>
          <p className="text-sm text-zinc-400 max-w-md mx-auto mt-1">
            Specify a default `docker-compose.yml` file in Settings to monitor services and manage project lifecycles.
          </p>
        </div>
      )}
    </div>
  );
};
