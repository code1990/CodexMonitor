import type { ReactNode } from "react";

type ChatPaneProps = {
  messagesNode: ReactNode;
  composerNode: ReactNode;
  messagesHidden?: boolean;
  className?: string;
};

export function ChatPane({
  messagesNode,
  composerNode,
  messagesHidden = false,
  className,
}: ChatPaneProps) {
  return (
    <div className={`chat-pane${className ? ` ${className}` : ""}`}>
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
