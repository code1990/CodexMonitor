import { describe, expect, it } from "vitest";

import { buildEbookPrdTasks } from "./ebookPrdAutomation";

describe("buildEbookPrdTasks", () => {
  it("sorts numbered chapters and constrains every task to one absolute path", () => {
    const tasks = buildEbookPrdTasks({
      inputDirectory: "D:\\ebook\\01章节正文",
      outputDirectory: "D:\\ebook\\02需求文档",
      productType: "量化研究工具",
      chapters: [
        { name: "10 第十章.md", path: "D:\\ebook\\01章节正文\\10 第十章.md" },
        { name: "02 第二章.md", path: "D:\\ebook\\01章节正文\\02 第二章.md" },
      ],
    });

    expect(tasks.map((task) => task.sourceFileName)).toEqual(["02 第二章.md", "10 第十章.md"]);
    expect(tasks[0].text).toContain("D:\\ebook\\01章节正文\\02 第二章.md");
    expect(tasks[0].text).toContain("只处理该章节内容");
    expect(tasks[0].text).toContain("不得依据上一章内容");
  });

  it("rejects chapter names without a numeric prefix", () => {
    expect(() =>
      buildEbookPrdTasks({
        inputDirectory: "D:\\ebook\\01章节正文",
        outputDirectory: "D:\\ebook\\02需求文档",
        chapters: [{ name: "序章.md", path: "D:\\ebook\\01章节正文\\序章.md" }],
      }),
    ).toThrow("numeric prefix");
  });
});
