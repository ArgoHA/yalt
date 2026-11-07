import { useCallback, useEffect, useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  createProject,
  getThumbnail,
  listRecentProjects,
  openProject,
  removeProject,
} from "./backend";
import type {
  ClassificationMode,
  ProjectSummary,
  RecentProject,
  KeypointSchema,
  TaskType,
} from "./types";
import { COCO_PERSON, parseKeypointNames } from "./editor/keypoints";
import { ProjectWorkspace } from "./components/ProjectWorkspace";
import yaltIcon from "../assets/brand/yalt-icon.svg";

const TASK_LABELS: Record<TaskType, string> = {
  detection: "Object detection",
  segmentation: "Polygon segmentation",
  keypoints: "Keypoint estimation",
  classification: "Image classification",
};

const TASK_SHORT: Record<TaskType, string> = {
  detection: "Detection",
  segmentation: "Segmentation",
  keypoints: "Keypoints",
  classification: "Classification",
};

const TASK_HINTS: Record<TaskType, string> = {
  detection: "Bounding boxes",
  segmentation: "Closed polygons",
  keypoints: "Boxes with named points",
  classification: "Classes per image",
};

function Icon({ name }: { name: "folder" | "plus" | "arrow" | "refresh" | "image" | "database" | "trash" }) {
  const paths = {
    folder: <path d="M3.5 6.5h6l2 2h9v10h-17zM3.5 6.5v-2h6l2 2" />,
    plus: <path d="M12 5v14M5 12h14" />,
    arrow: <path d="m9 5 7 7-7 7" />,
    refresh: <path d="M19 8a8 8 0 1 0 1 7M19 4v4h-4" />,
    image: <><rect x="3" y="4" width="18" height="16" rx="2" /><circle cx="8.5" cy="9" r="1.5" /><path d="m4 17 5-5 4 4 2-2 5 4" /></>,
    database: <><ellipse cx="12" cy="5" rx="8" ry="3" /><path d="M4 5v7c0 1.7 3.6 3 8 3s8-1.3 8-3V5M4 12v7c0 1.7 3.6 3 8 3s8-1.3 8-3v-7" /></>,
    trash: <><path d="M4 7h16M9 7V4h6v3M6.5 7l1 13h9l1-13M10 11v5M14 11v5" /></>,
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
    : <span className="thumb-empty"><Icon name="image" /></span>;
}

export default function App() {
  const [recents, setRecents] = useState<RecentProject[]>([]);
  const [project, setProject] = useState<ProjectSummary | null>(null);
  const [showCreate, setShowCreate] = useState(false);
  const [projectToRemove, setProjectToRemove] = useState<RecentProject | null>(null);
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

  const confirmRemoval = async () => {
    if (!projectToRemove) return;
    setBusy(true);
    setError(null);
    try {
      await removeProject(projectToRemove.rootPath);
      setProjectToRemove(null);
      await reloadRecents();
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
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
    <main className="home">
      <header className="topbar">
        <div className="home-inner">
          <div className="brand">
            <img src={yaltIcon} alt="" />
            <span className="brand-name">yalt</span>
            <span className="status" title="Images never leave your Mac. Every edit saves automatically."><i className="dot" />Local only</span>
          </div>
          <nav className="controls" aria-label="Project actions">
            <button className="btn" onClick={() => void chooseExisting()}>
              <Icon name="folder" />Open project
            </button>
            <button className="btn primary" onClick={() => { setError(null); setShowCreate(true); }}>
              <Icon name="plus" />New project
            </button>
          </nav>
        </div>
      </header>

      <section className="home-body">
        <div className="home-inner">
          <div className="sec-head">
            <h2>Recent projects</h2>
            <span className="sec-sub">Each project reopens exactly where you left it. Autosave is always on.</span>
          </div>

          {error && <div className="error-banner" role="alert">{error}<button onClick={() => setError(null)}>Dismiss</button></div>}

          <div className="project-grid" aria-busy={busy}>
            {recents.length === 0 ? (
              <button className="empty-projects" onClick={() => { setError(null); setShowCreate(true); }}>
                <Icon name="image" />
                <strong>Create your first project</strong>
                <span>Choose a folder of JPEG, PNG, WebP, or TIFF images. yalt indexes them in place without copying.</span>
              </button>
            ) : recents.map((recent) => (
              <article className="project-card-wrap" key={recent.rootPath}>
                <button
                  className="project-card"
                  disabled={!recent.available || busy}
                  onClick={() => void openRoot(recent.rootPath)}
                >
                  <span className="project-thumb">
                    <RecentPreview project={recent} />
                    {!recent.available && <span className="thumb-unavailable">Folder unavailable</span>}
                  </span>
                  <span className="project-meta">
                    <span className="project-title"><strong>{recent.name}</strong><span>{TASK_SHORT[recent.taskType]}</span></span>
                    <span className="project-path" title={recent.rootPath}>{recent.rootPath}</span>
                  </span>
                </button>
                <button
                  className="icon-button project-remove"
                  aria-label={`Remove ${recent.name}`}
                  title="Remove project"
                  disabled={busy}
                  onClick={() => { setError(null); setProjectToRemove(recent); }}
                >
                  <Icon name="trash" />
                </button>
              </article>
            ))}
          </div>
        </div>
      </section>

      {projectToRemove && (
        <RemoveProjectSheet
          project={projectToRemove}
          busy={busy}
          error={error}
          onCancel={() => { if (!busy) setProjectToRemove(null); }}
          onConfirm={confirmRemoval}
        />
      )}

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

function RemoveProjectSheet({
  project,
  busy,
  error,
  onCancel,
  onConfirm,
}: {
  project: RecentProject;
  busy: boolean;
  error: string | null;
  onCancel: () => void;
  onConfirm: () => Promise<void>;
}) {
  return (
    <div className="sheet-backdrop" onMouseDown={(event) => event.target === event.currentTarget && onCancel()}>
      <form
        className="sheet remove-sheet"
        role="dialog"
        aria-modal="true"
        aria-labelledby="remove-project-title"
        onSubmit={(event) => { event.preventDefault(); void onConfirm(); }}
      >
        <header className="sheet-head">
          <h2 id="remove-project-title">Remove “{project.name}”?</h2>
          {project.available ? (
            <p>yalt will delete the project’s hidden <code>.yalt</code> data and remove it from this screen. Your original images and every other source file will stay untouched.</p>
          ) : (
            <p>The source folder is unavailable, so yalt can remove this project from the list but cannot delete its hidden project data. Your original files will stay untouched.</p>
          )}
        </header>
        {project.available && <p className="remove-warning">Annotations, labels, history, and thumbnails stored by yalt will be permanently deleted. This cannot be undone.</p>}
        {error && <div className="sheet-error" role="alert">{error}</div>}
        <footer className="sheet-foot">
          <button type="button" className="btn" autoFocus onClick={onCancel} disabled={busy}>Cancel</button>
          <button className="btn danger" disabled={busy}>{busy ? "Removing…" : project.available ? "Remove project" : "Remove from list"}</button>
        </footer>
      </form>
    </div>
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
    keypointSchema: KeypointSchema | null;
  }) => Promise<void>;
}) {
  const [rootPath, setRootPath] = useState("");
  const [name, setName] = useState("");
  const [taskType, setTaskType] = useState<TaskType>("detection");
  const [classificationMode, setClassificationMode] = useState<ClassificationMode>("single");
  const [keypointNames, setKeypointNames] = useState("");
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
        className="sheet create-sheet"
        role="dialog"
        aria-modal="true"
        aria-labelledby="new-project-title"
        onSubmit={(event) => {
          event.preventDefault();
          if (rootPath && name.trim()) {
            const names = parseKeypointNames(keypointNames);
            // The COCO person preset keeps its skeleton; custom points start without one.
            const isCocoPerson = names.join(",") === COCO_PERSON.names.join(",");
            void onCreate({
              rootPath,
              name,
              taskType,
              classificationMode: taskType === "classification" ? classificationMode : null,
              keypointSchema: taskType === "keypoints" ? { names, skeleton: isCocoPerson ? COCO_PERSON.skeleton : [] } : null,
            });
          }
        }}
      >
        <header className="sheet-head"><h2 id="new-project-title">New project</h2><p>Choose one annotation task. It cannot change after annotations are created.</p></header>

        {error && <div className="sheet-error" role="alert">{error}</div>}

        <label className="field-label" htmlFor="project-name">Project name</label>
        <input id="project-name" className="input" autoFocus value={name} onChange={(event) => setName(event.target.value)} placeholder="Wildlife training set" />

        <span className="field-label">Task</span>
        <div className="task-options">
          {(Object.keys(TASK_LABELS) as TaskType[]).map((task) => (
            <label className={taskType === task ? "task-option selected" : "task-option"} key={task}>
              <input type="radio" name="task" checked={taskType === task} onChange={() => setTaskType(task)} />
              <span><strong>{TASK_LABELS[task]}</strong><small>{TASK_HINTS[task]}</small></span>
            </label>
          ))}
        </div>

        {taskType === "classification" && (
          <div className="mode-row">
            <span>Labels per image</span>
            <div className="seg" role="group" aria-label="Labels per image">
              {(["single", "multi"] as ClassificationMode[]).map((mode) => (
                <button type="button" aria-pressed={classificationMode === mode} onClick={() => setClassificationMode(mode)} key={mode}>
                  {mode === "single" ? "Single" : "Multiple"}
                </button>
              ))}
            </div>
          </div>
        )}

        {taskType === "keypoints" && <>
          <div className="field-row">
            <label className="field-label" htmlFor="keypoint-names">Keypoints per object</label>
            <button type="button" className="btn quiet" onClick={() => setKeypointNames(COCO_PERSON.names.join(", "))}>COCO person (17)</button>
          </div>
          <input id="keypoint-names" className="input" value={keypointNames} onChange={(event) => setKeypointNames(event.target.value)} placeholder="head, tail" />
          <p className="storage-explainer">Comma-separated, in export order. You can rename or add points later, or take them from an imported file.</p>
        </>}

        <span className="field-label">Image folder</span>
        <button className={`folder-picker${folderDropActive ? " drop-active" : ""}${rootPath ? " has-value" : ""}`} type="button" onClick={() => void chooseFolder()}>
          <Icon name="folder" /><span>{rootPath || "Choose a folder or drop one here…"}</span>
        </button>
        <p className="storage-explainer">Annotations will be stored in <code>.yalt/project.sqlite</code> inside this folder.</p>

        <footer className="sheet-foot">
          <button type="button" className="btn" onClick={onCancel}>Cancel</button>
          <button className="btn primary" disabled={!rootPath || !name.trim() || busy}>{busy ? "Creating…" : "Create project"}</button>
        </footer>
      </form>
    </div>
  );
}
