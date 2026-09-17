import { useTranslation } from "react-i18next";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import type { ModelImageSupport } from "../hooks/useModelImageSupport";

interface ModelImageSupportSelectProps {
  value: ModelImageSupport;
  onChange: (value: ModelImageSupport) => void;
  disabled?: boolean;
  ariaLabel?: string;
}

/**
 * 按模型声明图片输入能力的三态选择器：
 * 自动（不写声明，走内置名单/上游兜底）/ 支持 / 不支持。
 */
export function ModelImageSupportSelect({
  value,
  onChange,
  disabled,
  ariaLabel,
}: ModelImageSupportSelectProps) {
  const { t } = useTranslation();
  return (
    <Select
      value={value}
      onValueChange={(next) => onChange(next as ModelImageSupport)}
      disabled={disabled}
    >
      <SelectTrigger
        className="h-9 w-full"
        aria-label={
          ariaLabel ??
          t("providerForm.modelImageSupportHeader", { defaultValue: "图片" })
        }
      >
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        <SelectItem value="auto">
          {t("providerForm.modelImageSupportAuto", { defaultValue: "自动" })}
        </SelectItem>
        <SelectItem value="supported">
          {t("providerForm.modelImageSupportSupported", {
            defaultValue: "支持",
          })}
        </SelectItem>
        <SelectItem value="unsupported">
          {t("providerForm.modelImageSupportUnsupported", {
            defaultValue: "不支持",
          })}
        </SelectItem>
      </SelectContent>
    </Select>
  );
}
