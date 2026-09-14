import { useCallback, useEffect, useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  createProject,
  getThumbnail,
  listRecentProjects,
  openProject,
} from "./backend";
import type {
  ClassificationMode,
  ProjectSummary,
  RecentProject,
  TaskType,
} from "./types";
import { ProjectWorkspace } from "./components/ProjectWorkspace";
import yaltIcon from "../assets/brand/yalt-icon.svg";

const TASK_LABELS: Record<TaskType, string> = {
  detection: "Object detection",
  segmentation: "Polygon segmentation",
  classification: "Image classification",
};

function Icon({ name }: { name: "folder" | "plus" | "arrow" | "refresh" | "image" | "database" }) {
  const paths = {
    folder: <path d="M3.5 6.5h6l2 2h9v10h-17zM3.5 6.5v-2h6l2 2" />,
    plus: <path d="M12 5v14M5 12h14" />,
    arrow: <path d="m9 5 7 7-7 7" />,
    refresh: <path d="M19 8a8 8 0 1 0 1 7M19 4v4h-4" />,
    image: <><rect x="3" y="4" width="18" height="16" rx="2" /><circle cx="8.5" cy="9" r="1.5" /><path d="m4 17 5-5 4 4 2-2 5 4" /></>,
    database: <><ellipse cx="12" cy="5" rx="8" ry="3" /><path d="M4 5v7c0 1.7 3.6 3 8 3s8-1.3 8-3V5M4 12v7c0 1.7 3.6 3 8 3s8-1.3 8-3v-7" /></>,
  };
  return <svg aria-hidden="true" viewBox="0 0 24 24">{paths[name]}</svg>;
}

function errorMessage(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return "The operation could not be completed.";
}

function pathName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).at(-1) ?? "Untitled project";
}

function RecentPreview({ project }: { project: RecentProject }) {
  const [source, setSource] = useState<string | null>(null);

  useEffect(() => {
    if (!project.available || !project.previewImageId) {
      setSource(null);
      return;
    }
    let cancelled = false;
    getThumbnail(project.rootPath, project.previewImageId)
      .then((thumbnail) => { if (!cancelled) setSource(thumbnail); })
      .catch(() => { if (!cancelled) setSource(null); });
    return () => { cancelled = true; };
  }, [project.available, project.previewImageId, project.rootPath]);

  return source
    ? <img src={source} alt="" />
    : <span className="recent-preview-empty"><Icon name="image" /></span>;
}

export default function App() {
  const [recents, setRecents] = useState<RecentProject[]>([]);
  const [project, setProject] = useState<ProjectSummary | null>(null);
  const [showCreate, setShowCreate] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const reloadRecents = useCallback(async () => {
    try {
      setRecents(await listRecentProjects());
    } catch (reason) {
      setError(errorMessage(reason));
    }
  }, []);

  useEffect(() => {
    void reloadRecents();
  }, [reloadRecents]);

  const openRoot = async (rootPath: string) => {
    setBusy(true);
    setError(null);
    try {
      setProject(await openProject(rootPath));
      await reloadRecents();
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  };

  const chooseExisting = async () => {
    const selected = await openDialog({
      directory: true,
      multiple: false,
      title: "Open a yalt project folder",
    });
    if (typeof selected === "string") await openRoot(selected);
  };

  if (project) {
    return (
      <ProjectWorkspace
        project={project}
        onProjectChange={setProject}
        onClose={() => {
          setProject(null);
          void reloadRecents();
        }}
      />
    );
  }

  return (
    <main className="home-shell">
      <aside className="project-rail">
        <div className="wordmark"><img src={yaltIcon} alt="" />yalt</div>
        <nav className="rail-actions" aria-label="Project actions">
          <button className="rail-button primary" onClick={() => { setError(null); setShowCreate(true); }}>
            <Icon name="plus" />New project
          </button>
          <button className="rail-button" onClick={() => void chooseExisting()}>
            <Icon name="folder" />Open project
          </button>
        </nav>
        <div className="local-note">
          <Icon name="database" />
          <div><strong>Local only</strong><span>Images never leave your Mac.</span></div>
        </div>
      </aside>

      <section className="home-content">
        <header className="home-header">
          <div>
            <h1>Continue annotating</h1>
            <p>Each project reopens exactly where you left it.</p>
          </div>
          <span className="autosave-state"><span />Automatic saving is always on</span>
        </header>

        {error && <div className="error-banner" role="alert">{error}<button onClick={() => setError(null)}>Dismiss</button></div>}

        <div className="recent-list" aria-busy={busy}>
          {recents.length === 0 ? (
            <button className="empty-projects" onClick={() => { setError(null); setShowCreate(true); }}>
              <Icon name="image" />
              <strong>Create your first local project</strong>
              <span>Choose a folder of JPEG, PNG, WebP, or TIFF images. yalt indexes them without copying them.</span>
            </button>
          ) : recents.map((recent) => (
            <button
              className="recent-card"
              key={recent.rootPath}
              disabled={!recent.available || busy}
              onClick={() => void openRoot(recent.rootPath)}
            >
              <span className="recent-preview">
                <RecentPreview project={recent} />
                <span className="preview-task">{TASK_LABELS[recent.taskType]}</span>
                {!recent.available && <span className="preview-unavailable">Folder unavailable</span>}
              </span>
              <span className="recent-card-copy">
                <strong>{recent.name}</strong>
                <span title={recent.rootPath}>{recent.rootPath}</span>
              </span>
            </button>
          ))}
        </div>
      </section>

      {showCreate && (
        <CreateProjectSheet
          busy={busy}
          error={error}
          onCancel={() => setShowCreate(false)}
          onCreate={async (input) => {
            setBusy(true);
            setError(null);
            try {
              const created = await createProject(input);
              setShowCreate(false);
              setProject(created);
              await reloadRecents();
            } catch (reason) {
              setError(errorMessage(reason));
            } finally {
              setBusy(false);
            }
          }}
        />
      )}
    </main>
  );
}

function CreateProjectSheet({
  busy,
  error,
  onCancel,
  onCreate,
}: {
  busy: boolean;
  error: string | null;
  onCancel: () => void;
  onCreate: (input: {
    rootPath: string;
    name: string;
    taskType: TaskType;
    classificationMode: ClassificationMode | null;
  }) => Promise<void>;
}) {
  const [rootPath, setRootPath] = useState("");
  const [name, setName] = useState("");
  const [taskType, setTaskType] = useState<TaskType>("detection");
  const [classificationMode, setClassificationMode] = useState<ClassificationMode>("single");
  const [folderDropActive, setFolderDropActive] = useState(false);

  const useFolderPath = useCallback((path: string) => {
    setRootPath(path);
    setName((current) => current || pathName(path));
  }, []);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    try {
      void getCurrentWindow().onDragDropEvent((event) => {
        if (event.payload.type === "enter" || event.payload.type === "over") {
          setFolderDropActive(true);
        } else if (event.payload.type === "leave") {
          setFolderDropActive(false);
        } else {
          setFolderDropActive(false);
          const path = event.payload.paths[0];
          if (path) useFolderPath(path);
        }
      }).then((stopListening) => {
        if (disposed) stopListening();
        else unlisten = stopListening;
      }).catch(() => undefined);
    } catch {
      // Native Finder drops are only available inside the Tauri application.
    }
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [useFolderPath]);

  const chooseFolder = async () => {
    const selected = await openDialog({ directory: true, multiple: false, title: "Choose the image folder" });
    if (typeof selected === "string") useFolderPath(selected);
  };

  return (
    <div className="sheet-backdrop" onMouseDown={(event) => event.target === event.currentTarget && onCancel()}>
      <form
        className="create-sheet"
        role="dialog"
        aria-modal="true"
        aria-labelledby="new-project-title"
        onSubmit={(event) => {
          event.preventDefault();
          if (rootPath && name.trim()) {
            void onCreate({
              rootPath,
              name,
              taskType,
              classificationMode: taskType === "classification" ? classificationMode : null,
            });
          }
        }}
      >
        <header><h2 id="new-project-title">New project</h2><p>Choose one annotation task. It cannot change after annotations are created.</p></header>

        {error && <div className="sheet-error" role="alert">{error}</div>}

        <label className="field-label" htmlFor="project-name">Project name</label>
        <input id="project-name" autoFocus value={name} onChange={(event) => setName(event.target.value)} placeholder="Wildlife training set" />

        <span className="field-label">Task</span>
        <div className="task-options">
          {(Object.keys(TASK_LABELS) as TaskType[]).map((task) => (
            <label className={taskType === task ? "task-option selected" : "task-option"} key={task}>
              <input type="radio" name="task" checked={taskType === task} onChange={() => setTaskType(task)} />
              <span><strong>{TASK_LABELS[task]}</strong><small>{task === "detection" ? "Bounding boxes" : task === "segmentation" ? "Closed polygons" : "Classes per image"}</small></span>
            </label>
          ))}
        </div>

        {taskType === "classification" && (
          <div className="mode-row">
            <span>Labels per image</span>
            <div className="segmented-control">
              {(["single", "multi"] as ClassificationMode[]).map((mode) => (
                <button type="button" className={classificationMode === mode ? "active" : ""} aria-pressed={classificationMode === mode} onClick={() => setClassificationMode(mode)} key={mode}>
                  {mode === "single" ? "Single" : "Multiple"}
                </button>
              ))}
            </div>
          </div>
        )}

        <span className="field-label">Image folder</span>
        <button className={folderDropActive ? "folder-picker drop-active" : "folder-picker"} type="button" onClick={() => void chooseFolder()}>
          <Icon name="folder" /><span>{rootPath || "Choose a folder or drop one here…"}</span>
        </button>
        <p className="storage-explainer">Annotations will be stored in <code>.yalt/project.sqlite</code> inside this folder.</p>

        <footer>
          <button type="button" className="text-button" onClick={onCancel}>Cancel</button>
          <button className="action-button" disabled={!rootPath || !name.trim() || busy}>{busy ? "Creating…" : "Create project"}</button>
        </footer>
      </form>
    </div>
  );
}
