// 侧边栏公共入口 —— 按职责拆分到四个子文件，此处统一 re-export，
// 消费者 import 路径 "@/components/ui/sidebar" 无需变更。

export { useSidebar, SidebarProvider } from "@/components/ui/sidebar-context"

export {
  Sidebar,
  SidebarTrigger,
  SidebarRail,
  SidebarInset,
} from "@/components/ui/sidebar-root"

export {
  SidebarInput,
  SidebarHeader,
  SidebarFooter,
  SidebarSeparator,
  SidebarContent,
  SidebarGroup,
  SidebarGroupLabel,
  SidebarGroupAction,
  SidebarGroupContent,
} from "@/components/ui/sidebar-layout"

export {
  SidebarMenu,
  SidebarMenuItem,
  SidebarMenuButton,
  SidebarMenuAction,
  SidebarMenuBadge,
  SidebarMenuSkeleton,
  SidebarMenuSub,
  SidebarMenuSubItem,
  SidebarMenuSubButton,
} from "@/components/ui/sidebar-menu"
