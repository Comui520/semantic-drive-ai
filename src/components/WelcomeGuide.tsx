import { useState } from 'react'
import { STORAGE_KEY } from './welcomeStorage'

interface Step {
  icon: string
  title: string
  description: string
}

const steps: Step[] = [
  {
    icon: '🔍',
    title: '智能搜索',
    description: '用自然语言找到任何文件',
  },
  {
    icon: '💬',
    title: 'AI 助手',
    description: '像聊天一样管理文件',
  },
  {
    icon: '🛡️',
    title: '安全空间',
    description: '加密保护你的敏感文件',
  },
]


interface WelcomeGuideProps {
  onComplete: () => void
}

export default function WelcomeGuide({ onComplete }: WelcomeGuideProps) {
  const [current, setCurrent] = useState(0)
  const [exiting, setExiting] = useState(false)

  const isLast = current === steps.length - 1

  const finish = () => {
    localStorage.setItem(STORAGE_KEY, 'true')
    onComplete()
  }

  const handleNext = () => {
    if (isLast) {
      setExiting(true)
      setTimeout(() => finish(), 150)
    } else {
      setCurrent((c) => c + 1)
    }
  }

  const handleSkip = () => {
    setExiting(true)
    setTimeout(() => finish(), 150)
  }

  const handleBackdropClick = (e: React.MouseEvent) => {
    if (e.target === e.currentTarget) {
      // Do nothing; user must click "跳过" or complete the steps
    }
  }

  const step = steps[current]

  return (
    <div
      className={`fixed inset-0 z-[9999] flex items-center justify-center bg-black/70 backdrop-blur-sm transition-opacity duration-200 ${
        exiting ? 'opacity-0' : 'opacity-100'
      }`}
      onClick={handleBackdropClick}
    >
      <div className={`glass rounded-2xl w-[420px] max-w-[92vw] px-8 py-10 flex flex-col items-center text-center shadow-2xl shadow-deepsea-900/50 transition-all duration-200 ${
        exiting ? 'opacity-0 scale-95' : 'opacity-100 scale-100'
      }`}>
        {/* Step icon */}
        <div className="text-7xl mb-6 animate-fade-in" key={`icon-${current}`}>
          {step.icon}
        </div>

        {/* Title */}
        <h2
          className="text-2xl font-bold text-primary mb-3 animate-fade-in"
          key={`title-${current}`}
        >
          {step.title}
        </h2>

        {/* Description */}
        <p
          className="text-secondary text-base mb-10 animate-fade-in"
          key={`desc-${current}`}
        >
          {step.description}
        </p>

        {/* Dot indicators */}
        <div className="flex items-center gap-2.5 mb-8">
          {steps.map((_, i) => (
            <button
              key={i}
              onClick={() => setCurrent(i)}
              className={`w-2.5 h-2.5 rounded-full transition-all duration-300 ${
                i === current
                  ? 'bg-accent-cyan w-7'
                  : 'bg-silver-500/40 hover:bg-silver-400/60'
              }`}
              aria-label={`跳转到第 ${i + 1} 步`}
            />
          ))}
        </div>

        {/* Next / Start button */}
        <button
          onClick={handleNext}
          className={`w-full py-3 rounded-xl text-base font-semibold transition-all duration-200 ${
            isLast
              ? 'bg-accent-teal text-white hover:brightness-110 active:scale-[0.98]'
              : 'bg-accent-cyan text-deepsea-900 hover:brightness-110 active:scale-[0.98]'
          }`}
        >
          {isLast ? '开始使用' : '下一步'}
        </button>

        {/* Skip link */}
        <button
          onClick={handleSkip}
          className="mt-4 text-sm text-muted hover:text-secondary transition-colors duration-200"
        >
          跳过
        </button>
      </div>
    </div>
  )
}
