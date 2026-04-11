/** @vitest-environment jsdom */
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { useRef, useState } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { isMobilePlatform } from "../../../utils/platformPaths";
import { Composer } from "./Composer";
import type {
  AppOption,
  AppMention,
  ComposerSendIntent,
  FollowUpMessageBehavior,
} from "../../../types";
import type { RuntimeAutomationController } from "@app/runtime/runtimeHost";

const tauriMocks = vi.hoisted(() => ({
  pickDirectoryMock: vi.fn(async () => null),
  pickTextFileMock: vi.fn(async () => null),
  readTextFileMock: vi.fn(async () => ""),
  listTextFilesInDirectoryMock: vi.fn(async () => []),
  writeTextFileMock: vi.fn(async () => undefined),
}));

vi.mock("../../../services/dragDrop", () => ({
  subscribeWindowDragDrop: vi.fn(() => () => {}),
}));

vi.mock("../../../services/tauri", () => ({
  pickDirectory: tauriMocks.pickDirectoryMock,
  pickTextFile: tauriMocks.pickTextFileMock,
  readTextFile: tauriMocks.readTextFileMock,
  listTextFilesInDirectory: tauriMocks.listTextFilesInDirectoryMock,
  writeTextFile: tauriMocks.writeTextFileMock,
}));

vi.mock("@tauri-apps/api/core", () => ({
  convertFileSrc: (path: string) => `tauri://${path}`,
}));

vi.mock("../../../utils/platformPaths", async () => {
  const actual = await vi.importActual<typeof import("../../../utils/platformPaths")>(
    "../../../utils/platformPaths",
  );
  return {
    ...actual,
    isMobilePlatform: vi.fn(() => false),
  };
});

type HarnessProps = {
  onSend: (
    text: string,
    images: string[],
    appMentions?: AppMention[],
    submitIntent?: ComposerSendIntent,
  ) => void;
  apps?: AppOption[];
  isProcessing?: boolean;
  followUpMessageBehavior?: FollowUpMessageBehavior;
  steerAvailable?: boolean;
  selectedServiceTier?: "fast" | "flex" | null;
  automationController?: RuntimeAutomationController | null;
};

function ComposerHarness({
  onSend,
  apps = [],
  isProcessing = false,
  followUpMessageBehavior = "queue",
  steerAvailable = false,
  selectedServiceTier = null,
  automationController = null,
}: HarnessProps) {
  const [draftText, setDraftText] = useState("");
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);

  return (
    <Composer
      onSend={onSend}
      onStop={() => {}}
      canStop={false}
      isProcessing={isProcessing}
      appsEnabled={true}
      steerAvailable={steerAvailable}
      followUpMessageBehavior={followUpMessageBehavior}
      composerFollowUpHintEnabled={true}
      collaborationModes={[]}
      selectedCollaborationModeId={null}
      onSelectCollaborationMode={() => {}}
      models={[]}
      selectedModelId={null}
      onSelectModel={() => {}}
      reasoningOptions={[]}
      selectedEffort={null}
      onSelectEffort={() => {}}
      selectedServiceTier={selectedServiceTier}
      reasoningSupported={false}
      accessMode="current"
      onSelectAccessMode={() => {}}
      skills={[]}
      apps={apps}
      prompts={[]}
      files={[]}
      draftText={draftText}
      onDraftChange={setDraftText}
      textareaRef={textareaRef}
      dictationEnabled={false}
      automationController={automationController}
    />
  );
}

describe("Composer send triggers", () => {
  afterEach(() => {
    cleanup();
    vi.mocked(isMobilePlatform).mockReturnValue(false);
    tauriMocks.pickDirectoryMock.mockReset();
    tauriMocks.pickTextFileMock.mockReset();
    tauriMocks.readTextFileMock.mockReset();
    tauriMocks.listTextFilesInDirectoryMock.mockReset();
    tauriMocks.writeTextFileMock.mockReset();
    tauriMocks.pickDirectoryMock.mockResolvedValue(null);
    tauriMocks.pickTextFileMock.mockResolvedValue(null);
    tauriMocks.readTextFileMock.mockResolvedValue("");
    tauriMocks.listTextFilesInDirectoryMock.mockResolvedValue([]);
    tauriMocks.writeTextFileMock.mockResolvedValue(undefined);
    vi.restoreAllMocks();
  });

  it("sends once on Enter", () => {
    const onSend = vi.fn();
    render(<ComposerHarness onSend={onSend} />);

    const textarea = screen.getByRole("textbox");
    fireEvent.change(textarea, { target: { value: "hello world" } });
    fireEvent.keyDown(textarea, { key: "Enter" });

    expect(onSend).toHaveBeenCalledTimes(1);
    expect(onSend).toHaveBeenCalledWith("hello world", [], undefined, "default");
  });

  it("sends once on send-button click", () => {
    const onSend = vi.fn();
    render(<ComposerHarness onSend={onSend} />);

    const textarea = screen.getByRole("textbox");
    fireEvent.change(textarea, { target: { value: "from button" } });
    fireEvent.click(screen.getByLabelText("Send"));

    expect(onSend).toHaveBeenCalledTimes(1);
    expect(onSend).toHaveBeenCalledWith("from button", [], undefined, "default");
  });

  it("shows the fast-mode indicator when enabled", () => {
    const onSend = vi.fn();
    render(<ComposerHarness onSend={onSend} selectedServiceTier="fast" />);

    expect(screen.getByLabelText("Fast mode enabled")).toBeTruthy();
  });

  it("blurs the textarea after Enter send on mobile", () => {
    vi.mocked(isMobilePlatform).mockReturnValue(true);
    const onSend = vi.fn();
    const blurSpy = vi.spyOn(HTMLTextAreaElement.prototype, "blur");
    render(<ComposerHarness onSend={onSend} />);

    const textarea = screen.getByRole("textbox");
    fireEvent.change(textarea, { target: { value: "dismiss keyboard" } });
    fireEvent.keyDown(textarea, { key: "Enter" });

    expect(onSend).toHaveBeenCalledTimes(1);
    expect(onSend).toHaveBeenCalledWith(
      "dismiss keyboard",
      [],
      undefined,
      "default",
    );
    expect(blurSpy).toHaveBeenCalledTimes(1);
  });

  it("sends explicit app mentions when an app autocomplete item is selected", () => {
    const onSend = vi.fn();
    render(
      <ComposerHarness
        onSend={onSend}
        apps={[
          {
            id: "connector_calendar",
            name: "Calendar App",
            description: "Calendar integration",
            isAccessible: true,
          },
        ]}
      />,
    );

    const textarea = screen.getByRole("textbox");
    fireEvent.change(textarea, { target: { value: "$cal" } });
    fireEvent.keyDown(textarea, { key: "Tab" });
    fireEvent.keyDown(textarea, { key: "Enter" });

    expect(onSend).toHaveBeenCalledTimes(1);
    expect(onSend).toHaveBeenCalledWith(
      "$calendar-app",
      [],
      [{ name: "Calendar App", path: "app://connector_calendar" }],
      "default",
    );
  });

  it("uses queue by default while processing when follow-up behavior is queue", () => {
    const onSend = vi.fn();
    render(
      <ComposerHarness
        onSend={onSend}
        isProcessing={true}
        followUpMessageBehavior="queue"
        steerAvailable={true}
      />,
    );

    const textarea = screen.getByRole("textbox");
    fireEvent.change(textarea, { target: { value: "queue this" } });
    fireEvent.keyDown(textarea, { key: "Enter" });

    expect(onSend).toHaveBeenCalledTimes(1);
    expect(onSend).toHaveBeenCalledWith("queue this", [], undefined, "queue");
  });

  it("uses opposite follow-up behavior on Shift+Ctrl+Enter while processing", () => {
    const onSend = vi.fn();
    render(
      <ComposerHarness
        onSend={onSend}
        isProcessing={true}
        followUpMessageBehavior="queue"
        steerAvailable={true}
      />,
    );

    const textarea = screen.getByRole("textbox");
    fireEvent.change(textarea, { target: { value: "steer this" } });
    fireEvent.keyDown(textarea, { key: "Enter", shiftKey: true, ctrlKey: true });

    expect(onSend).toHaveBeenCalledTimes(1);
    expect(onSend).toHaveBeenCalledWith("steer this", [], undefined, "steer");
  });

  it("falls back to queue when steer is selected but unavailable", () => {
    const onSend = vi.fn();
    render(
      <ComposerHarness
        onSend={onSend}
        isProcessing={true}
        followUpMessageBehavior="steer"
        steerAvailable={false}
      />,
    );

    const textarea = screen.getByRole("textbox");
    fireEvent.change(textarea, { target: { value: "queue fallback" } });
    fireEvent.keyDown(textarea, { key: "Enter" });

    expect(
      screen.getByText(
        "Default: Queue (Steer unavailable). Both Enter and Shift+Ctrl+Enter will queue this message.",
      ),
    ).toBeTruthy();
    expect(onSend).toHaveBeenCalledTimes(1);
    expect(onSend).toHaveBeenCalledWith("queue fallback", [], undefined, "queue");
  });

  it("treats Shift+Ctrl+Enter like normal send when not processing", () => {
    const onSend = vi.fn();
    render(
      <ComposerHarness
        onSend={onSend}
        isProcessing={false}
        followUpMessageBehavior="queue"
        steerAvailable={true}
      />,
    );

    const textarea = screen.getByRole("textbox");
    fireEvent.change(textarea, { target: { value: "normal shortcut send" } });
    fireEvent.keyDown(textarea, { key: "Enter", shiftKey: true, ctrlKey: true });

    expect(onSend).toHaveBeenCalledTimes(1);
    expect(onSend).toHaveBeenCalledWith(
      "normal shortcut send",
      [],
      undefined,
      "default",
    );
  });

  it("does not queue on Tab while processing", () => {
    const onSend = vi.fn();
    render(
      <ComposerHarness
        onSend={onSend}
        isProcessing={true}
        followUpMessageBehavior="queue"
        steerAvailable={true}
      />,
    );

    const textarea = screen.getByRole("textbox");
    fireEvent.change(textarea, { target: { value: "tab no send" } });
    fireEvent.keyDown(textarea, { key: "Tab" });

    expect(onSend).not.toHaveBeenCalled();
  });

  it("imports txt tasks and auto-dispatches the first line when enabled", async () => {
    const onSend = vi.fn();
    const { container } = render(<ComposerHarness onSend={onSend} />);

    const fileInput = container.querySelector(
      ".composer-automation-file",
    ) as HTMLInputElement | null;
    expect(fileInput).toBeTruthy();

    const file = new File(["task one\ntask two"], "tasks.txt", {
      type: "text/plain",
    });
    Object.defineProperty(file, "text", {
      value: vi.fn().mockResolvedValue("task one\ntask two"),
    });

    await act(async () => {
      fireEvent.change(fileInput!, {
        target: {
          files: [file],
        },
      });
    });

    await act(async () => {
      fireEvent.click(screen.getByLabelText("Auto"));
    });

    expect(await screen.findByText("task one")).toBeTruthy();
    expect(onSend).toHaveBeenCalledTimes(1);
    expect(onSend).toHaveBeenCalledWith("task one", [], undefined, "default");
  });

  it("syncs prompt txt selection into the runtime automation controller", async () => {
    const onSend = vi.fn();
    const automationController: RuntimeAutomationController = {
      enabled: false,
      promptEnabled: false,
      promptText: "",
      promptSourceName: null,
      tasks: [],
      summary: {
        sourceName: null,
        total: 0,
        pending: 0,
        running: 0,
        completed: 0,
        failed: 0,
        timedOut: 0,
        hasTerminalIssue: false,
        activeTaskId: null,
      },
      setEnabled: vi.fn(),
      setPromptEnabled: vi.fn(),
      setPromptText: vi.fn(),
      setPromptSourceName: vi.fn(),
      importTasks: vi.fn(),
      appendTasks: vi.fn(),
      importTasksFromText: vi.fn(),
      clearAutomationTasks: vi.fn(),
    };
    const { container } = render(
      <ComposerHarness onSend={onSend} automationController={automationController} />,
    );

    const fileInputs = container.querySelectorAll(
      ".composer-automation-file",
    ) as NodeListOf<HTMLInputElement>;
    const promptFileInput = fileInputs[1];
    expect(promptFileInput).toBeTruthy();

    const file = new File(["system prefix"], "prompt.txt", {
      type: "text/plain",
    });
    Object.defineProperty(file, "text", {
      value: vi.fn().mockResolvedValue("system prefix"),
    });

    await act(async () => {
      fireEvent.change(promptFileInput, {
        target: {
          files: [file],
        },
      });
    });

    expect(automationController.setPromptText).toHaveBeenCalledWith("system prefix");
    expect(automationController.setPromptSourceName).toHaveBeenCalledWith("prompt.txt");
    expect(automationController.setPromptEnabled).toHaveBeenCalledWith(true);
  });

  it("loads prompt txt from a clipboard path copied from model output", async () => {
    const onSend = vi.fn();
    tauriMocks.readTextFileMock.mockResolvedValue("system prefix");
    const clipboardReadText = vi.fn().mockResolvedValue("‪C:\\Users\\htzl\\Desktop\\json.txt");
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { readText: clipboardReadText },
    });

    render(<ComposerHarness onSend={onSend} />);

    await act(async () => {
      fireEvent.click(screen.getByText("Prompt TXT"));
    });

    expect(clipboardReadText).toHaveBeenCalledTimes(1);
    expect(tauriMocks.readTextFileMock).toHaveBeenCalledWith(
      "C:\\Users\\htzl\\Desktop\\json.txt",
    );
    expect(screen.getByText("Prompt: json.txt")).toBeTruthy();
    expect(screen.getByLabelText("Use prompt TXT")).toHaveProperty("checked", true);
  });

  it("keeps Clear enabled when only prompt/download state has values and clears them all", async () => {
    const onSend = vi.fn();
    tauriMocks.readTextFileMock.mockResolvedValue("system prefix");
    tauriMocks.pickDirectoryMock.mockResolvedValue("C:\\Users\\htzl\\Pictures");
    const clipboardReadText = vi.fn().mockResolvedValue("C:\\Users\\htzl\\Desktop\\flow.txt");
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { readText: clipboardReadText },
    });

    render(<ComposerHarness onSend={onSend} />);

    await act(async () => {
      fireEvent.click(screen.getByText("Prompt TXT"));
    });
    await act(async () => {
      fireEvent.click(screen.getByText("Download Dir"));
    });

    const clearButton = screen.getByText("Clear");
    expect(clearButton.hasAttribute("disabled")).toBe(false);

    await act(async () => {
      fireEvent.click(clearButton);
    });

    expect(screen.getByText("Prompt: -")).toBeTruthy();
    expect(screen.getByText("Download: -")).toBeTruthy();
    expect(screen.getByLabelText("Use prompt TXT")).toHaveProperty("checked", false);
    expect(screen.getByLabelText("Auto")).toHaveProperty("checked", false);
  });

  it("does not reapply stale prompt controller values after Clear", async () => {
    const onSend = vi.fn();
    const automationController: RuntimeAutomationController = {
      enabled: false,
      promptEnabled: false,
      promptText: "",
      promptSourceName: null,
      tasks: [],
      summary: {
        sourceName: null,
        total: 0,
        pending: 0,
        running: 0,
        completed: 0,
        failed: 0,
        timedOut: 0,
        hasTerminalIssue: false,
        activeTaskId: null,
      },
      setEnabled: vi.fn(),
      setPromptEnabled: vi.fn(),
      setPromptText: vi.fn(),
      setPromptSourceName: vi.fn(),
      importTasks: vi.fn(),
      appendTasks: vi.fn(),
      importTasksFromText: vi.fn(),
      clearAutomationTasks: vi.fn(),
    };

    const { rerender } = render(
      <ComposerHarness onSend={onSend} automationController={automationController} />,
    );

    const promptFileInput = document.querySelectorAll(".composer-automation-file")[1] as
      | HTMLInputElement
      | undefined;
    expect(promptFileInput).toBeTruthy();

    const file = new File(["system prefix"], "prompt.txt", {
      type: "text/plain",
    });
    Object.defineProperty(file, "text", {
      value: vi.fn().mockResolvedValue("system prefix"),
    });

    await act(async () => {
      fireEvent.change(promptFileInput!, {
        target: {
          files: [file],
        },
      });
    });

    await act(async () => {
      fireEvent.click(screen.getByText("Clear"));
    });

    rerender(<ComposerHarness onSend={onSend} automationController={automationController} />);

    const promptEnabledCalls = vi.mocked(automationController.setPromptEnabled).mock.calls;
    expect(promptEnabledCalls[promptEnabledCalls.length - 1]).toEqual([false]);
    expect(promptEnabledCalls.some((args) => args[0] === true)).toBe(true);
    expect(promptEnabledCalls.slice(-1)[0][0]).toBe(false);
  });
});
