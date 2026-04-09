import type { ReactNode } from "react";

type ChatPaneProps = {
  topNode?: ReactNode;
  messagesNode: ReactNode;
  composerNode: ReactNode;
  messagesHidden?: boolean;
  className?: string;
};

export function ChatPane({
  topNode = null,
  messagesNode,
  composerNode,
  messagesHidden = false,
  className,
}: ChatPaneProps) {
  return (
    <div className={`chat-pane${className ? ` ${className}` : ""}`}>
      {topNode ? <div className="chat-pane-top">{topNode}</div> : null}
      <div className="chat-pane-messages" aria-hidden={messagesHidden}>
        {messagesHidden ? null : messagesNode}
      </div>
      {composerNode ? (
        <div className="chat-pane-composer">
          {composerNode}
        </div>
      ) : null}
    </div>
  );
}
