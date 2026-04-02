import { useEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode } from "react";

type ChatPaneProps = {
  topNode?: ReactNode;
  messagesNode: ReactNode;
  composerNode: ReactNode;
  className?: string;
};

export function ChatPane({ topNode = null, messagesNode, composerNode, className }: ChatPaneProps) {
  const composerRef = useRef<HTMLDivElement | null>(null);
  const [composerHeight, setComposerHeight] = useState(0);

  useEffect(() => {
    if (!composerNode) {
      setComposerHeight(0);
      return;
    }

    const node = composerRef.current;
    if (!node) {
      return;
    }

    const updateComposerHeight = () => {
      setComposerHeight(Math.ceil(node.getBoundingClientRect().height));
    };

    updateComposerHeight();

    const observer = new ResizeObserver(() => {
      updateComposerHeight();
    });
    observer.observe(node);

    return () => {
      observer.disconnect();
    };
  }, [composerNode]);

  const paneStyle = useMemo(
    () =>
      ({
        ["--composer-overlay-height" as string]: `${composerHeight}px`,
      }) satisfies CSSProperties,
    [composerHeight],
  );

  return (
    <div className={`chat-pane${className ? ` ${className}` : ""}`} style={paneStyle}>
      {topNode ? <div className="chat-pane-top">{topNode}</div> : null}
      <div className="chat-pane-messages">{messagesNode}</div>
      {composerNode ? (
        <div className="chat-pane-composer" ref={composerRef}>
          {composerNode}
        </div>
      ) : null}
    </div>
  );
}
