import { useCallback, useEffect, useRef } from "react"

import {
  DRAFT_DEBOUNCE_MS,
  DRAFT_FORCE_MS,
  clearDraft,
  draftsEqual,
  writeDraft,
} from "@/lib/draft"

export interface AutosaveDraftOptions<T> {
  /** 草稿 key（见 lib/draft 的 postDraftKey / pageDraftKey） */
  storageKey: string
  /** 当前编辑器快照 */
  data: T
  /** 服务端基线快照；null = 尚未加载完成（此时不写，避免空表单覆盖已有草稿） */
  baseline: T | null
  /** 额外开关（如加载中/加载失败时置 false） */
  enabled?: boolean
}

/**
 * 编辑器自动保存到 localStorage：
 * - 输入防抖 DRAFT_DEBOUNCE_MS（停止输入 1.5s 落一次）；
 * - 持续输入时每 DRAFT_FORCE_MS（30s）强制落一次（防抖窗口被不断重置也不会一直不存）；
 * - 页面隐藏（pagehide / visibilitychange→hidden）与组件卸载（路由跳转）时同步补落一次；
 * - 快照与 baseline 完全一致（含恢复到原文）时不写；写得进去与否交给 writeDraft 静默处理。
 *
 * 返回 markSaved()：保存成功（后端已确认）后调用——立即清除该 key 并复位内部脏标记，
 * 防止卸载时的补落把刚清掉的草稿又写回来。
 */
export function useAutosaveDraft<T>({
  storageKey,
  data,
  baseline,
  enabled = true,
}: AutosaveDraftOptions<T>): { markSaved: () => void } {
  const dataRef = useRef(data)
  const dirtyRef = useRef(false)
  /** 上次成功落盘的时刻；0 = 从未写过 */
  const lastWriteRef = useRef(0)

  useEffect(() => {
    dataRef.current = data
    const dirty = enabled && baseline !== null && !draftsEqual(data, baseline)
    dirtyRef.current = dirty
    if (!dirty) return

    // 常态走防抖；距上次落盘接近强制间隔时按剩余时间提前触发
    const since = Date.now() - lastWriteRef.current
    const delay = Math.max(0, Math.min(DRAFT_DEBOUNCE_MS, DRAFT_FORCE_MS - since))
    const timer = setTimeout(() => {
      if (writeDraft(storageKey, dataRef.current)) lastWriteRef.current = Date.now()
    }, delay)
    return () => clearTimeout(timer)
  }, [storageKey, data, baseline, enabled])

  // 页面隐藏/关闭或组件卸载时补落一次（同步写，尽量赶在页面卸载前完成）
  useEffect(() => {
    const flush = () => {
      if (!dirtyRef.current) return
      if (writeDraft(storageKey, dataRef.current)) lastWriteRef.current = Date.now()
    }
    const onVisibility = () => {
      if (document.visibilityState === "hidden") flush()
    }
    window.addEventListener("pagehide", flush)
    document.addEventListener("visibilitychange", onVisibility)
    return () => {
      window.removeEventListener("pagehide", flush)
      document.removeEventListener("visibilitychange", onVisibility)
      flush()
    }
  }, [storageKey])

  const markSaved = useCallback(() => {
    clearDraft(storageKey)
    dirtyRef.current = false
    lastWriteRef.current = 0
  }, [storageKey])

  return { markSaved }
}
