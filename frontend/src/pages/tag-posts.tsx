import { useParams } from "react-router-dom"

import { PostFeed } from "@/components/post-feed"

export default function TagPostsPage() {
  const { name = "" } = useParams()
  return <PostFeed tag={name} heading={`标签：${name}`} />
}
