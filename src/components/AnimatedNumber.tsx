import { useEffect } from "react";
import { motion, useMotionValue, useSpring, useTransform } from "framer-motion";

interface Props {
  value: number;
  decimals?: number;
  className?: string;
}

/** 平滑滚动的数字，用于大屏速度/连接数展示 */
export function AnimatedNumber({ value, decimals = 2, className }: Props) {
  const mv = useMotionValue(0);
  const spring = useSpring(mv, { stiffness: 120, damping: 24, mass: 0.7 });
  const text = useTransform(spring, (v) => v.toFixed(decimals));

  useEffect(() => {
    mv.set(value);
  }, [value, mv]);

  return <motion.span className={className}>{text}</motion.span>;
}
