import type { AutoTaskImportInput } from "../hooks/useAutoTaskRunner";

type EbookChapterFile = {
  name: string;
  path: string;
};

export type EbookPrdTaskInput = {
  inputDirectory: string;
  outputDirectory: string;
  chapters: EbookChapterFile[];
  productType?: string;
};

const chapterNumber = (fileName: string) => {
  const match = fileName.match(/^\s*(\d+)/);
  return match ? Number.parseInt(match[1], 10) : null;
};

export function buildEbookPrdTasks({
  inputDirectory,
  outputDirectory,
  chapters,
  productType = "",
}: EbookPrdTaskInput): AutoTaskImportInput[] {
  const normalizedInputDirectory = inputDirectory.trim();
  const normalizedOutputDirectory = outputDirectory.trim();
  if (!normalizedInputDirectory || !normalizedOutputDirectory) {
    throw new Error("Chapter and output directories are required.");
  }
  if (chapters.length === 0) {
    throw new Error("The selected chapter directory contains no Markdown files.");
  }

  const invalidChapter = chapters.find((chapter) => chapterNumber(chapter.name) === null);
  if (invalidChapter) {
    throw new Error(`Chapter filename requires a numeric prefix: ${invalidChapter.name}`);
  }

  const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });
  return [...chapters]
    .sort((left, right) => collator.compare(left.name, right.name))
    .map((chapter, index) => ({
      lineNumber: index + 1,
      sourceFileName: chapter.name,
      sourcePath: chapter.path,
      exportFileName: chapter.name,
      text: [
        "本轮仅处理一个章节。",
        `输入目录：${normalizedInputDirectory}`,
        `输出目录：${normalizedOutputDirectory}`,
        `当前目标：${chapter.name}`,
        productType.trim() ? `产品形态：${productType.trim()}` : "",
        "",
        "必须重新读取并独立理解以下文件全文：",
        chapter.path,
        "",
        "只处理该章节内容，只在输出目录生成与该章节同名的需求文档。",
        "不得依据上一章内容推断、复用、合并章节或处理下一章。",
        "完成本章后停止，等待下一条自动指令。",
      ]
        .filter(Boolean)
        .join("\n"),
    }));
}
