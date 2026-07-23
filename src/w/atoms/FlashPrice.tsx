import type React from 'react'
import { useEffect, useMemo, useRef, useState } from 'react'

interface FlashPriceProps {
  price: number | string
  upColorClass?: string
  downColorClass?: string
  defaultColorClass?: string
  flashDuration?: number
}

const formatPrice = (price: number | string): string => {
  const str = price.toString()
  const parts = str.split('.')
  // Format the integer part using toLocaleString
  // Handle potential negative sign correctly by parsing, or Number() handles it.
  const integerPart = parts[0]
  const formattedInteger =
    integerPart === '' || integerPart === '-'
      ? integerPart
      : Number(integerPart).toLocaleString('en-US')

  if (parts.length > 1) {
    return `${formattedInteger}.${parts[1]}`
  }
  return formattedInteger
}

export const FlashPrice: React.FC<FlashPriceProps> = ({
  price,
  upColorClass = 'text-emerald-400',
  downColorClass = 'text-rose-400',
  defaultColorClass = '',
  flashDuration = 500
}) => {
  const [prevPrice, setPrevPrice] = useState<number | string>(price)
  const [changeIndex, setChangeIndex] = useState<number>(-1)
  const [isUp, setIsUp] = useState<boolean>(true)
  const timeoutRef = useRef<NodeJS.Timeout | null>(null)

  const currentFormatted = useMemo(() => formatPrice(price), [price])
  // We need to track the PREVIOUS formatted string to carry over the comparison state
  // properly across renders when price updates.
  const [prevFormatted, setPrevFormatted] = useState<string>(formatPrice(price))

  useEffect(() => {
    if (price === prevPrice) return

    const currStr = formatPrice(price)
    // prevFormatted is stale in closure if we don't include it in dep array,
    // but we want the value from STATE that corresponds to 'prevPrice'.
    // Actually, since we update them together, using the state value is correct.

    // 找到第一个不同的位置
    let diffIndex = 0
    const minLen = Math.min(prevFormatted.length, currStr.length)

    let foundDiff = false
    for (let i = 0; i < minLen; i++) {
      if (prevFormatted[i] !== currStr[i]) {
        diffIndex = i
        foundDiff = true
        break
      }
    }

    // 如果长度不同，从长度变化处开始
    if (!foundDiff && prevFormatted.length !== currStr.length) {
      diffIndex = minLen
    }

    setChangeIndex(diffIndex)
    setIsUp(Number(price) > Number(prevPrice))
    setPrevPrice(price)
    setPrevFormatted(currStr)

    // 清除之前的定时器
    if (timeoutRef.current) {
      clearTimeout(timeoutRef.current)
    }

    // 设置新的定时器来重置颜色
    timeoutRef.current = setTimeout(() => {
      setChangeIndex(-1)
    }, flashDuration)

    return () => {
      if (timeoutRef.current) {
        clearTimeout(timeoutRef.current)
      }
    }
  }, [price, prevPrice, prevFormatted, flashDuration])

  const { unchangedPart, changedPart } = useMemo(() => {
    const priceString = currentFormatted
    if (changeIndex === -1 || changeIndex >= priceString.length) {
      return { unchangedPart: priceString, changedPart: '' }
    }

    return {
      unchangedPart: priceString.slice(0, changeIndex),
      changedPart: priceString.slice(changeIndex)
    }
  }, [currentFormatted, changeIndex])

  return (
    <>
      <span className={defaultColorClass}>{unchangedPart}</span>
      <span
        className={changeIndex !== -1 ? (isUp ? upColorClass : downColorClass) : defaultColorClass}
      >
        {changedPart}
      </span>
    </>
  )
}
