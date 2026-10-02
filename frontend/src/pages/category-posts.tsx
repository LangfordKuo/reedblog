import { useParams } from "react-router-dom"

import { PostFeed } from "@/components/post-feed"

export default function CategoryPostsPage() {
  const { name = "" } = useParams()
  return <PostFeed category={name} heading={`分类：${name}`} />
}
