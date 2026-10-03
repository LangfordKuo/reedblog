import { useCallback, useEffect, useState, type FormEvent } from "react"
import { Loader2Icon, PencilIcon, PlusIcon, Trash2Icon } from "lucide-react"
import { toast } from "sonner"

import { ConfirmDialog } from "@/components/confirm-dialog"
import { BlockSpinner } from "@/components/spinner"
import { Button } from "@/components/ui/button"
import { Card, CardContent } from "@/components/ui/card"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { api, errorMessage } from "@/lib/api"
import type { Category, Tag } from "@/lib/types"

type Item = Category | Tag

/** 分类管理 / 标签管理共用组件 */
export function TaxonomyAdmin({ kind }: { kind: "category" | "tag" }) {
  const isCategory = kind === "category"
  const noun = isCategory ? "分类" : "标签"
  const [items, setItems] = useState<Item[] | null>(null)
  const [newName, setNewName] = useState("")
  const [creating, setCreating] = useState(false)
  const [editing, setEditing] = useState<Item | null>(null)
  const [editName, setEditName] = useState("")
  const [savingEdit, setSavingEdit] = useState(false)
  const [deleting, setDeleting] = useState<Item | null>(null)
  const [deleteLoading, setDeleteLoading] = useState(false)

  const load = useCallback(() => {
    const req = isCategory ? api.admin.categories() : api.admin.tags()
    req.then(setItems).catch((e) => {
      setItems([])
      toast.error(errorMessage(e))
    })
  }, [isCategory])

  useEffect(load, [load])

  const handleCreate = async (e: FormEvent) => {
    e.preventDefault()
    const name = newName.trim()
    if (!name || creating) return
    setCreating(true)
    try {
      const created = isCategory
        ? await api.admin.createCategory(name)
        : await api.admin.createTag(name)
      setItems((prev) => [created, ...(prev ?? [])])
      setNewName("")
      toast.success(`已创建${noun}「${name}」`)
    } catch (err) {
      toast.error(errorMessage(err, { duplicate_name: `${noun}名称已存在` }))
    } finally {
      setCreating(false)
    }
  }

  const openEdit = (item: Item) => {
    setEditing(item)
    setEditName(item.name)
  }

  const handleRename = async () => {
    if (!editing) return
    const name = editName.trim()
    if (!name) {
      toast.error("名称不能为空")
      return
    }
    setSavingEdit(true)
    try {
      const updated = isCategory
        ? await api.admin.updateCategory(editing.id, name)
        : await api.admin.updateTag(editing.id, name)
      setItems((prev) => (prev ?? []).map((it) => (it.id === updated.id ? updated : it)))
      setEditing(null)
      toast.success("已重命名")
    } catch (err) {
      toast.error(errorMessage(err, { duplicate_name: `${noun}名称已存在` }))
    } finally {
      setSavingEdit(false)
    }
  }

  const handleDelete = async () => {
    if (!deleting) return
    setDeleteLoading(true)
    try {
      if (isCategory) await api.admin.deleteCategory(deleting.id)
      else await api.admin.deleteTag(deleting.id)
      setItems((prev) => (prev ?? []).filter((it) => it.id !== deleting.id))
      toast.success(`已删除${noun}「${deleting.name}」`)
      setDeleting(null)
    } catch (err) {
      toast.error(errorMessage(err, { in_use: `该${noun}下仍有文章，无法删除` }))
      setDeleting(null)
    } finally {
      setDeleteLoading(false)
    }
  }

  return (
    <div className="flex flex-col gap-6">
      <div>
        <h1 className="text-xl font-bold">{noun}管理</h1>
        <p className="text-sm text-muted-foreground">
          共 {items?.length ?? 0} 个{noun}；仍有文章的{noun}无法删除。
        </p>
      </div>

      <Card>
        <CardContent>
          <form onSubmit={handleCreate} className="flex flex-wrap gap-2">
            <Input
              value={newName}
              onChange={(e) => setNewName(e.target.value)}
              placeholder={`新${noun}名称`}
              className="max-w-xs"
              maxLength={50}
            />
            <Button type="submit" disabled={creating || !newName.trim()}>
              {creating ? <Loader2Icon className="animate-spin" /> : <PlusIcon />}
              新建
            </Button>
          </form>
        </CardContent>
      </Card>

      {items === null ? (
        <BlockSpinner />
      ) : items.length === 0 ? (
        <div className="py-12 text-center text-sm text-muted-foreground">暂无{noun}</div>
      ) : (
        <Card>
          <CardContent>
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>名称</TableHead>
                  <TableHead>文章数</TableHead>
                  <TableHead className="text-right">操作</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {items.map((it) => (
                  <TableRow key={it.id}>
                    <TableCell className="font-medium">{it.name}</TableCell>
                    <TableCell>{it.post_count}</TableCell>
                    <TableCell>
                      <div className="flex justify-end gap-1">
                        <Button
                          variant="ghost"
                          size="icon"
                          aria-label="重命名"
                          onClick={() => openEdit(it)}
                        >
                          <PencilIcon />
                        </Button>
                        <Button
                          variant="ghost"
                          size="icon"
                          aria-label="删除"
                          className="text-destructive hover:text-destructive"
                          onClick={() => setDeleting(it)}
                        >
                          <Trash2Icon />
                        </Button>
                      </div>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </CardContent>
        </Card>
      )}

      <Dialog open={editing !== null} onOpenChange={(o) => !o && !savingEdit && setEditing(null)}>
        <DialogContent className="sm:max-w-sm">
          <DialogHeader>
            <DialogTitle>重命名{noun}</DialogTitle>
            <DialogDescription>为「{editing?.name}」设置新名称</DialogDescription>
          </DialogHeader>
          <div className="grid gap-1.5">
            <Label htmlFor="rename-input">名称</Label>
            <Input
              id="rename-input"
              value={editName}
              onChange={(e) => setEditName(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") handleRename()
              }}
              maxLength={50}
            />
          </div>
          <DialogFooter>
            <Button variant="outline" disabled={savingEdit} onClick={() => setEditing(null)}>
              取消
            </Button>
            <Button disabled={savingEdit} onClick={handleRename}>
              {savingEdit && <Loader2Icon className="animate-spin" />}
              保存
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <ConfirmDialog
        open={deleting !== null}
        onOpenChange={(o) => !o && setDeleting(null)}
        title={`删除${noun}`}
        description={
          // 目标为空（弹窗关闭/删除完成）时不渲染正文，避免把 undefined 拼进 DOM 文案
          deleting ? `确定删除「${deleting.name}」吗？此操作不可撤销。` : ""
        }
        loading={deleteLoading}
        onConfirm={handleDelete}
      />
    </div>
  )
}
