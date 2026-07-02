import {
  ASSIGNABLE_MENUS,
  type AssignableMenu,
  type Permission,
} from "@/lib/permissions";
import { cn } from "@/lib/utils";

// 权限键 → 中文名（子项提示「需先启用父项」时复用）
const MENU_LABEL = new Map<Permission, string>(
  ASSIGNABLE_MENUS.map((m) => [m.key, m.label]),
);

// 按 ASSIGNABLE_MENUS 声明顺序规范化输出，保证存储/展示稳定、与后端 to_storage 一致。
function normalize(set: Set<Permission>): Permission[] {
  return ASSIGNABLE_MENUS.filter((m) => set.has(m.key)).map((m) => m.key);
}

// 勾选/取消单个菜单：取消父项（view_assets）时级联移除依赖它的子项（manage_assets），
// 与后端 set_permissions「manage_assets ⇒ 需含 view_assets」校验呼应，避免非法组合。
function toggle(
  current: readonly Permission[],
  menu: AssignableMenu,
  checked: boolean,
): Permission[] {
  const set = new Set(current);
  if (checked) {
    set.add(menu.key);
  } else {
    set.delete(menu.key);
    for (const child of ASSIGNABLE_MENUS) {
      if (child.parent === menu.key) set.delete(child.key);
    }
  }
  return normalize(set);
}

type Props = {
  value: Permission[];
  onChange: (next: Permission[]) => void;
  disabled?: boolean;
};

// 纯受控菜单勾选器：自身不发请求，仅在 value 上做增删并回调 onChange。
// 有 parent 的项（manage_assets）缩进为子项；父项未勾时该子项 disabled，
// 且由 toggle 级联从 value 移除（父取消即子取消）。
export function MenuPermissionEditor({ value, onChange, disabled }: Props) {
  return (
    <div role="group" aria-label="菜单权限" className="grid gap-2">
      {ASSIGNABLE_MENUS.map((menu) => {
        const parentUnchecked = menu.parent
          ? !value.includes(menu.parent)
          : false;
        const rowDisabled = Boolean(disabled) || parentUnchecked;
        const checked = value.includes(menu.key) && !parentUnchecked;
        return (
          <label
            key={menu.key}
            className={cn(
              "flex items-center gap-2 text-sm",
              menu.parent && "pl-6",
              rowDisabled ? "text-muted-foreground" : "text-foreground",
              rowDisabled ? "cursor-not-allowed" : "cursor-pointer",
            )}
          >
            <input
              type="checkbox"
              className="size-4 cursor-pointer accent-primary align-middle disabled:cursor-not-allowed"
              checked={checked}
              disabled={rowDisabled}
              onChange={(e) => onChange(toggle(value, menu, e.target.checked))}
            />
            {menu.label}
            {parentUnchecked && menu.parent && (
              <span className="text-xs text-muted-foreground">
                （需先启用“{MENU_LABEL.get(menu.parent)}”）
              </span>
            )}
          </label>
        );
      })}
    </div>
  );
}
