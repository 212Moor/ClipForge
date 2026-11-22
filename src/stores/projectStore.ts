import { create } from "zustand";

interface ProjectUiState {
  activeProjectId: string | null;
  draftVodUrl: string;
  setActiveProjectId: (id: string | null) => void;
  setDraftVodUrl: (url: string) => void;
}

export const useProjectStore = create<ProjectUiState>((set) => ({
  activeProjectId: null,
  draftVodUrl: "",
  setActiveProjectId: (id) => set({ activeProjectId: id }),
  setDraftVodUrl: (url) => set({ draftVodUrl: url }),
}));
