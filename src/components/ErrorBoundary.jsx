import {Component} from 'react';

/**
 * 顶层错误边界：任何子树在渲染期抛异常时，显示可读的错误而不是整页白屏。
 * 开发时也把栈打出来，方便定位；点「重新加载」清掉内存里的坏状态。
 */
export default class ErrorBoundary extends Component {
    constructor(props) {
        super(props);
        this.state = {error: null};
    }

    static getDerivedStateFromError(error) {
        return {error};
    }

    componentDidCatch(error, info) {
        console.error('[NeuralKB] 渲染异常', error, info);
    }

    render() {
        const {error} = this.state;
        if (!error) return this.props.children;
        return (
            <div className="crash">
                <div className="crash-card">
                    <div className="crash-mark">!</div>
                    <h1>界面出了点问题</h1>
                    <p className="crash-lead">已经拦下这个错误，数据没有丢失。可以重新加载再试一次。</p>
                    <pre className="crash-detail">{String(error?.message || error)}</pre>
                    <div className="crash-actions">
                        <button className="btn primary" onClick={() => window.location.reload()}>
                            重新加载
                        </button>
                        <button
                            className="btn ghost"
                            onClick={() => this.setState({error: null})}
                        >
                            再试一次
                        </button>
                    </div>
                </div>
            </div>
        );
    }
}
